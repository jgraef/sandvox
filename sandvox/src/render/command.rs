use std::{
    any::TypeId,
    marker::PhantomData,
};

use bevy_ecs::{
    entity::Entity,
    query::{
        QueryState,
        ROQueryItem,
        ReadOnlyQueryData,
    },
    resource::Resource,
    schedule::SystemSet,
    system::{
        Query,
        ReadOnlySystemParam,
        ResMut,
        SystemParam,
        SystemParamItem,
        SystemState,
    },
    world::World,
};
use bevy_utils::TypeIdMap;

use crate::{
    ecs::{
        plugin::WorldBuilder,
        schedule,
    },
    render::{
        RenderSystems,
        pass::RenderPass,
    },
};

pub trait RenderCommand {
    type Param: SystemParam + 'static;
    type ViewQuery: ReadOnlyQueryData;
    type ItemQuery: ReadOnlyQueryData;

    fn render<'w>(
        param: SystemParamItem<'w, '_, Self::Param>,
        render_pass: &mut RenderPass<'w>,
        view: ROQueryItem<'w, '_, Self::ViewQuery>,
        items: Query<'w, '_, Self::ItemQuery>,
    );
}

struct RenderCommandState<C>
where
    C: RenderCommand,
{
    state: SystemState<C::Param>,
    view: QueryState<C::ViewQuery>,
    item: QueryState<C::ItemQuery>,
}

impl<C> RenderFunction for RenderCommandState<C>
where
    C: RenderCommand + 'static,
    C::Param: ReadOnlySystemParam,
{
    fn prepare(&mut self, world: &World) {
        self.view.update_archetypes(world);
        self.item.update_archetypes(world);
    }

    fn render<'w>(&mut self, world: &'w World, render_pass: &mut RenderPass<'w>, view: Entity) {
        let param = self.state.get(world);

        let view = self.view.get_manual(world, view).unwrap_or_else(|error| {
            todo!("handle error: {error}");
        });

        let items = self.item.query_manual(world);
        C::render(param, render_pass, view, items);
    }
}

pub trait AddRenderCommand {
    fn add_render_command<P, C: RenderCommand>(&mut self) -> &mut Self
    where
        P: 'static,
        C: RenderCommand + 'static,
        C::Param: ReadOnlySystemParam;
}

impl AddRenderCommand for WorldBuilder {
    fn add_render_command<P, C>(&mut self) -> &mut Self
    where
        P: 'static,
        C: RenderCommand + 'static,
        C::Param: ReadOnlySystemParam,
    {
        let render_command_state = RenderCommandState::<C> {
            state: SystemState::new(&mut self.world),
            view: self.world.query(),
            item: self.world.query(),
        };

        let mut render_commands = self.world.resource_mut::<RenderFunctions<P>>();
        render_commands.insert(render_command_state);

        self
    }
}

pub trait RenderFunction: Send + Sync + 'static {
    fn prepare(&mut self, world: &World);

    fn render<'w>(&mut self, world: &'w World, render_pass: &mut RenderPass<'w>, view: Entity);
}

#[derive(derive_more::Debug, Resource)]
pub(super) struct RenderFunctions<P> {
    #[debug(skip)]
    functions: Vec<Box<dyn RenderFunction>>,
    by_type_id: TypeIdMap<RenderFunctionId<P>>,
}

impl<P> RenderFunctions<P> {
    fn insert<F>(&mut self, render_function: F) -> RenderFunctionId<P>
    where
        F: RenderFunction,
    {
        let id = RenderFunctionId {
            index: self.functions.len(),
            _marker: PhantomData,
        };

        self.functions.push(Box::new(render_function));
        self.by_type_id.insert(TypeId::of::<F>(), id);

        id
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut dyn RenderFunction> {
        self.functions.iter_mut().map(|f| &mut **f)
    }
}

impl<P> Default for RenderFunctions<P> {
    fn default() -> Self {
        Self {
            functions: Default::default(),
            by_type_id: Default::default(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct RenderFunctionId<P> {
    index: usize,
    _marker: PhantomData<fn() -> P>,
}

impl<P> Clone for RenderFunctionId<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for RenderFunctionId<P> {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
pub struct PrepareRenderFunctions;

pub(super) fn prepare_render_functions<P>(
    mut render_functions: ResMut<RenderFunctions<P>>,
    world: &World,
) where
    P: 'static,
{
    for function in &mut render_functions.functions {
        function.prepare(world);
    }
}
