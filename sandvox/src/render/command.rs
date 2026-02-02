use std::{
    any::{
        TypeId,
        type_name,
    },
    marker::PhantomData,
};

use bevy_ecs::{
    entity::Entity,
    query::{
        ROQueryItem,
        ReadOnlyQueryData,
    },
    resource::Resource,
    system::{
        DynParamBuilder,
        DynSystemParam,
        ParamBuilder,
        ParamSet,
        ParamSetBuilder,
        Query,
        Res,
        StaticSystemParam,
        SystemParam,
        SystemParamBuilder,
        SystemParamItem,
    },
};
use bevy_utils::TypeIdMap;

use crate::{
    ecs::plugin::WorldBuilder,
    render::pass::RenderPass,
};

pub trait RenderFunction: Send + Sync + 'static {
    type Param: SystemParam + 'static;
    type ViewQuery: ReadOnlyQueryData;
    type ItemQuery: ReadOnlyQueryData;

    fn render(
        &self,
        param: SystemParamItem<Self::Param>,
        render_pass: &mut RenderPass<'_>,
        view: ROQueryItem<Self::ViewQuery>,
        items: Query<Self::ItemQuery>,
    );
}

#[derive(derive_more::Debug)]
pub struct RenderFunctions<'w, 's, P>
where
    P: 'static,
{
    #[debug(skip)]
    registry: Res<'w, Registry<P>>,

    #[debug(skip)]
    params: ParamSet<'w, 's, Vec<DynSystemParam<'static, 'static>>>,
}

impl<'w, 's, F> RenderFunctions<'w, 's, F> {
    pub fn render(&mut self, render_pass: &mut RenderPass, view: Entity) {
        let mut functions = self.registry.functions.iter();

        self.params.for_each(move |param| {
            let function = functions.next().unwrap();
            function.render(&mut *render_pass, view, param);
        });
    }
}

#[doc(hidden)]
pub struct FetchState<P>
where
    P: 'static,
{
    registry: <Res<'static, Registry<P>> as SystemParam>::State,
    params:
        <ParamSet<'static, 'static, Vec<DynSystemParam<'static, 'static>>> as SystemParam>::State,
}

unsafe impl<P> SystemParam for RenderFunctions<'_, '_, P> {
    type State = FetchState<P>;
    type Item<'w, 's> = RenderFunctions<'w, 's, P>;

    fn init_state(world: &mut bevy_ecs::world::World) -> Self::State {
        let registry = world.get_resource_or_init::<Registry<P>>();

        let params_builder = ParamSetBuilder(
            registry
                .functions
                .iter()
                .map(|function| function.build_system_param())
                .collect::<Vec<_>>(),
        );

        FetchState {
            registry: <Res<Registry<P>> as SystemParam>::init_state(world),
            params: params_builder.build(world),
        }
    }

    fn init_access(
        state: &Self::State,
        system_meta: &mut bevy_ecs::system::SystemMeta,
        component_access_set: &mut bevy_ecs::query::FilteredAccessSet,
        world: &mut bevy_ecs::world::World,
    ) {
        <Res<'static, Registry<P>> as SystemParam>::init_access(
            &state.registry,
            system_meta,
            component_access_set,
            world,
        );

        <ParamSet<'static, 'static, Vec<DynSystemParam<'static, 'static>>> as SystemParam>::init_access(&state.params, system_meta, component_access_set, world);
    }

    fn apply(
        state: &mut Self::State,
        system_meta: &bevy_ecs::system::SystemMeta,
        world: &mut bevy_ecs::world::World,
    ) {
        <Res<'static, Registry<P>> as SystemParam>::apply(&mut state.registry, system_meta, world);
        <ParamSet<'static, 'static, Vec<DynSystemParam<'static, 'static>>> as SystemParam>::apply(
            &mut state.params,
            system_meta,
            world,
        );
    }

    fn queue(
        state: &mut Self::State,
        system_meta: &bevy_ecs::system::SystemMeta,
        mut world: bevy_ecs::world::DeferredWorld,
    ) {
        <Res<'static, Registry<P>> as SystemParam>::queue(
            &mut state.registry,
            system_meta,
            world.reborrow(),
        );
        <ParamSet<'static, 'static, Vec<DynSystemParam<'static, 'static>>> as SystemParam>::queue(
            &mut state.params,
            system_meta,
            world,
        );
    }

    unsafe fn validate_param(
        state: &mut Self::State,
        system_meta: &bevy_ecs::system::SystemMeta,
        world: bevy_ecs::world::unsafe_world_cell::UnsafeWorldCell,
    ) -> Result<(), bevy_ecs::system::SystemParamValidationError> {
        unsafe {
            <Res<'static, Registry<P>> as SystemParam>::validate_param(
                &mut state.registry,
                system_meta,
                world,
            )?;
            <ParamSet<'static, 'static, Vec<DynSystemParam<'static, 'static>>> as SystemParam>::validate_param(&mut state.params, system_meta, world)?;
        }
        Ok(())
    }

    unsafe fn get_param<'world, 'state>(
        state: &'state mut Self::State,
        system_meta: &bevy_ecs::system::SystemMeta,
        world: bevy_ecs::world::unsafe_world_cell::UnsafeWorldCell<'world>,
        change_tick: bevy_ecs::component::Tick,
    ) -> Self::Item<'world, 'state> {
        unsafe {
            // SAFETY:
            // - We initialized the access for each parameter in `init_access`, so the
            //   caller ensures we have access to any world data needed by each param.
            // - The caller ensures this was the world used to initialize our state, and we
            //   used that world to initialize parameter states

            let registry = <Res<'static, Registry<P>> as SystemParam>::get_param(
                &mut state.registry,
                system_meta,
                world,
                change_tick,
            );
            let params = <ParamSet<'static, 'static, Vec<DynSystemParam<'static, 'static>>> as SystemParam>::get_param(&mut state.params, system_meta, world, change_tick);

            RenderFunctions { registry, params }
        }
    }
}

#[derive(Resource)]
struct Registry<P> {
    functions: Vec<Box<dyn DynRenderFunction>>,
    by_type_id: TypeIdMap<RenderFunctionId<P>>,
}

impl<P> Registry<P> {
    fn insert<F>(&mut self, render_function: F) -> RenderFunctionId<P>
    where
        F: RenderFunction,
    {
        let id = RenderFunctionId {
            index: self.functions.len(),
            _marker: PhantomData,
        };

        self.functions
            .push(Box::new(render_function) as Box<dyn DynRenderFunction>);
        self.by_type_id.insert(TypeId::of::<F>(), id);

        id
    }
}

impl<P> Default for Registry<P> {
    fn default() -> Self {
        Self {
            functions: Default::default(),
            by_type_id: Default::default(),
        }
    }
}

trait DynRenderFunction: Send + Sync + 'static {
    fn build_system_param(&self) -> DynParamBuilder<'static>;
    fn render(&self, render_pass: &mut RenderPass, view: Entity, param: DynSystemParam);
}

impl<F> DynRenderFunction for F
where
    F: RenderFunction,
{
    fn build_system_param(&self) -> DynParamBuilder<'static> {
        DynParamBuilder::new::<(
            StaticSystemParam<F::Param>,
            Query<F::ViewQuery>,
            Query<F::ItemQuery>,
            //Query<NameOrEntity>,
        )>(ParamBuilder)
    }

    fn render(&self, render_pass: &mut RenderPass, view: Entity, param: DynSystemParam) {
        let (param, views, items) = param
            .downcast::<(
                StaticSystemParam<F::Param>,
                Query<F::ViewQuery>,
                Query<F::ItemQuery>,
                //Query<NameOrEntity>,
            )>()
            .unwrap();

        // note: sometimes the view query doesn't match because the rendering function
        // does need data that's not available yet. we can just skip the whole render
        // function then.

        /*let view = views.get_inner(view).unwrap_or_else(|error| {
            let name = names.get(view).unwrap();
            panic!(
                "Could not get view '{name}' for render function `{}`: {error}",
                type_name::<F>()
            );
        });*/

        if let Ok(view) = views.get_inner(view) {
            <F as RenderFunction>::render(self, param.into_inner(), render_pass, view, items);
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

pub trait AddRenderFunction {
    fn add_render_function<P, F>(&mut self, function: F) -> &mut Self
    where
        P: 'static,
        F: RenderFunction + 'static;
}

impl AddRenderFunction for WorldBuilder {
    fn add_render_function<P, F>(&mut self, function: F) -> &mut Self
    where
        P: 'static,
        F: RenderFunction + 'static,
    {
        tracing::debug!(
            phase = type_name::<P>(),
            function = type_name::<F>(),
            "adding render function"
        );

        let mut registry = self.world.get_resource_or_init::<Registry<P>>();
        registry.insert(function);

        self
    }
}
