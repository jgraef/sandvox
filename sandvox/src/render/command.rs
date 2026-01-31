use std::any::TypeId;

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

impl<C> Draw for RenderCommandState<C>
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
    fn add_render_command<C: RenderCommand>(&mut self)
    where
        C: RenderCommand + 'static,
        C::Param: ReadOnlySystemParam;
}

impl AddRenderCommand for WorldBuilder {
    fn add_render_command<C>(&mut self)
    where
        C: RenderCommand + 'static,
        C::Param: ReadOnlySystemParam,
    {
        let render_command_state = RenderCommandState::<C> {
            state: SystemState::new(&mut self.world),
            view: self.world.query(),
            item: self.world.query(),
        };

        let mut render_commands = self.world.resource_mut::<DrawFunctions>();
        render_commands.insert(render_command_state);
    }
}

pub trait Draw: Send + Sync + 'static {
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
struct DrawFunctions {
    #[debug(skip)]
    draw_functions: Vec<Box<dyn Draw>>,
    by_type_id: TypeIdMap<DrawFunctionId>,
}

impl DrawFunctions {
    fn insert<D>(&mut self, draw_function: D) -> DrawFunctionId
    where
        D: Draw,
    {
        let id = DrawFunctionId(self.draw_functions.len());
        self.draw_functions.push(Box::new(draw_function));
        self.by_type_id.insert(TypeId::of::<D>(), id);
        id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct DrawFunctionId(usize);
