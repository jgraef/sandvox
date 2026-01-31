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
    system::{
        ReadOnlySystemParam,
        SystemParam,
        SystemParamItem,
        SystemState,
    },
    world::World,
};
use bevy_utils::TypeIdMap;

use crate::ecs::plugin::WorldBuilder;

pub trait RenderCommand {
    type Param: SystemParam + 'static;
    type ViewQuery: ReadOnlyQueryData;
    type ItemQuery: ReadOnlyQueryData;

    fn render<'w>(
        param: SystemParamItem<'w, '_, Self::Param>,
        render_pass: &mut wgpu::RenderPass<'w>,
        view: ROQueryItem<'w, '_, Self::ViewQuery>,
        item: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
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

    fn render<'w>(
        &mut self,
        world: &'w World,
        render_pass: &mut wgpu::RenderPass<'w>,
        view: Entity,
        item: Entity,
    ) {
        let param = self.state.get(world);

        let view = self.view.get_manual(world, view).unwrap_or_else(|error| {
            todo!("handle error: {error}");
        });

        let item = self.item.get_manual(world, item).ok();
        C::render(param, render_pass, view, item);
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

    fn render<'w>(
        &mut self,
        world: &'w World,
        render_pass: &mut wgpu::RenderPass<'w>,
        view: Entity,
        item: Entity,
    );
}

#[derive(derive_more::Debug, Resource)]
pub(super) struct RenderFunctions<P> {
    #[debug(skip)]
    draw_functions: Vec<Box<dyn RenderFunction>>,
    by_type_id: TypeIdMap<RenderFunctionId<P>>,
}

impl<P> RenderFunctions<P> {
    fn insert<F>(&mut self, render_function: F) -> RenderFunctionId<P>
    where
        F: RenderFunction,
    {
        let id = RenderFunctionId {
            index: self.draw_functions.len(),
            _marker: PhantomData,
        };

        self.draw_functions.push(Box::new(render_function));
        self.by_type_id.insert(TypeId::of::<F>(), id);

        id
    }
}

impl<P> Default for RenderFunctions<P> {
    fn default() -> Self {
        Self {
            draw_functions: Default::default(),
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
