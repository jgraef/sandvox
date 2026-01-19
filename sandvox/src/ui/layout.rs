use std::ops::{
    Deref,
    DerefMut,
};

use bevy_ecs::{
    component::Component,
    entity::{
        Entity,
        EntityHashSet,
    },
    hierarchy::{
        ChildOf,
        Children,
    },
    query::{
        Changed,
        Has,
        Or,
        QueryData,
        With,
        Without,
    },
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        common_conditions::any_match_filter,
    },
    system::{
        Commands,
        Local,
        ParamSet,
        Populated,
        Query,
        Res,
        StaticSystemParam,
        SystemParam,
    },
};
use nalgebra::Vector2;
use taffy::{
    AvailableSpace,
    CacheTree,
    LayoutPartialTree,
    NodeId,
    Size,
    TraversePartialTree,
};

use crate::{
    ecs::{
        plugin::WorldBuilder,
        schedule,
    },
    ui::{
        UiSurface,
        UiSystems,
    },
};

pub trait LeafMeasure: Send + Sync + 'static {
    type Data: SystemParam + Send + Sync + 'static;
    type Node: QueryData + Send + Sync + 'static;

    fn measure(
        &self,
        leaf: &mut <Self::Node as QueryData>::Item<'_, '_>,
        data: &mut <Self::Data as SystemParam>::Item<'_, '_>,
        known_dimensions: Size<Option<f32>>,
        available_space: Size<AvailableSpace>,
    ) -> Size<f32>;
}

pub(super) fn setup_layout_systems<L>(builder: &mut WorldBuilder, layout_config: LayoutConfig<L>)
where
    L: LeafMeasure,
{
    builder.insert_resource(layout_config).add_systems(
        schedule::Render,
        (
            initialize_layout_components,
            purge_dirty_cache_entries,
            layout_trees::<L>
                .run_if(any_match_filter::<Changed<LayoutCache>>)
                .after(initialize_layout_components)
                .after(purge_dirty_cache_entries)
                .in_set(UiSystems::Layout),
        ),
    );
}

/// System that computes the layout of all UI trees
///
/// # TODO
///
/// make this run only if something changes. Preferably only compute subtrees
///   if they changed. taffy caches them, but we do know when stuff changes.
///   Could work like the transform hierarchy.
///
/// This might help too:
///
/// ```no_run
/// .run_if(any_match_filter::<Or<(Changed<Style>, Changed<LeafMeasure>)>>)
/// ```
fn layout_trees<L>(
    mut tree: Tree<L>,
    roots: Populated<(Entity, &UiSurface), (With<Style>, Without<ChildOf>)>,
) where
    L: LeafMeasure,
{
    for (entity, surface) in roots.iter() {
        tree.compute_root_layout(entity, surface.size);
    }
}

fn initialize_layout_components(
    nodes: Populated<
        (Entity, Has<LayoutCache>, Has<UnroundedLayout>),
        (
            With<Style>,
            Or<(Without<LayoutCache>, Without<UnroundedLayout>)>,
        ),
    >,
    mut commands: Commands,
) {
    for (entity, has_cache, has_unrounded_layout) in nodes {
        let mut entity = commands.entity(entity);
        if !has_cache {
            entity.insert(LayoutCache::default());
        }
        if !has_unrounded_layout {
            entity.insert(UnroundedLayout::default());
        }
    }
}

fn purge_dirty_cache_entries(
    mut params: ParamSet<(
        Populated<Entity, Changed<LayoutCache>>,
        Query<(&mut LayoutCache, Option<&ChildOf>)>,
    )>,
    mut purged: Local<EntityHashSet>,
    mut queue: Local<Vec<Entity>>,
) {
    assert!(purged.is_empty());
    assert!(queue.is_empty());

    for entity in params.p0() {
        queue.push(entity);
    }

    let mut caches = params.p1();
    while let Some(entity) = queue.pop() {
        if let Ok((mut cache, child_of)) = caches.get_mut(entity) {
            cache.0.clear();

            if let Some(child_of) = child_of
                && !purged.contains(&child_of.0)
            {
                queue.push(child_of.0);
            }
        }

        purged.insert(entity);
    }

    purged.clear();
}

#[derive(Clone, Debug, Default, Component)]
pub struct Style(style_not_send_sync_patch::Style);

impl From<taffy::Style> for Style {
    fn from(value: taffy::Style) -> Self {
        Self(style_not_send_sync_patch::Style(value))
    }
}

impl Deref for Style {
    type Target = taffy::Style;

    fn deref(&self) -> &Self::Target {
        &self.0.0
    }
}

impl DerefMut for Style {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0.0
    }
}

#[derive(Debug, Default, Component)]
pub struct LayoutCache(taffy::Cache);

impl LayoutCache {
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[derive(Clone, Debug, Default, Component)]
struct UnroundedLayout(taffy::Layout);

#[derive(Clone, Debug, Component, derive_more::Deref)]
pub struct RoundedLayout(taffy::Layout);

#[derive(Clone, Copy, Debug, Component)]
struct DebugLabel(&'static str);

// note: the derive macro for `QueryData` doesn't like it when we put the trait
// bound `L: Leaf` into a where clause. we think this is a bug with bevy_ecs
// that should be reported.
#[derive(Debug, QueryData)]
#[query_data(mutable)]
struct Node<L: LeafMeasure> {
    style: &'static Style,
    unrounded_layout: &'static mut UnroundedLayout,
    rounded_layout: Option<&'static mut RoundedLayout>,
    cache: &'static mut LayoutCache,
    debug_label: Option<&'static DebugLabel>,
    children: Option<&'static Children>,
    leaf: Option<L::Node>,
}

#[inline(always)]
fn node_id_to_entity(node_id: NodeId) -> Entity {
    Entity::from_bits(node_id.into())
}

#[inline(always)]
fn entity_to_node_id(entity: Entity) -> NodeId {
    entity.to_bits().into()
}

#[derive(Debug, Resource)]
pub(super) struct LayoutConfig<L> {
    pub leaf_measure: L,
}

#[derive(SystemParam)]
pub(super) struct Tree<'w, 's, L>
where
    L: LeafMeasure,
{
    nodes: Query<'w, 's, Node<L>>,
    leaf_data: StaticSystemParam<'w, 's, <L as LeafMeasure>::Data>,
    layout_config: Res<'w, LayoutConfig<L>>,
    commands: Commands<'w, 's>,
}

impl<'w, 's, L> Tree<'w, 's, L>
where
    L: LeafMeasure,
{
    fn compute_root_layout(&mut self, root: Entity, surface_size: Vector2<f32>) {
        let root = entity_to_node_id(root);

        let available_size = Size {
            width: AvailableSpace::Definite(surface_size.x),
            height: AvailableSpace::Definite(surface_size.y),
        };

        taffy::compute_root_layout(self, root, available_size);
        taffy::round_layout(self, root);
    }

    #[allow(dead_code)]
    pub fn print(&self, root: Entity) {
        let root = entity_to_node_id(root);
        taffy::print_tree(self, root);
    }

    fn get_style_for_node(&self, node_id: NodeId) -> &taffy::Style {
        &*self.nodes.get(node_id_to_entity(node_id)).unwrap().style
    }

    fn compute_uncached_child_layout(
        &mut self,
        node_id: taffy::NodeId,
        inputs: taffy::LayoutInput,
    ) -> taffy::LayoutOutput {
        let entity = node_id_to_entity(node_id);
        tracing::trace!(?entity, "computing tree layout for node");

        let mut node = self.nodes.get_mut(entity).unwrap();

        if let Some(leaf) = &mut node.leaf {
            taffy::compute_leaf_layout(
                inputs,
                &**node.style,
                |_calc_ptr, _parent_size| 0.0,
                |known_dimensions, available_space| {
                    self.layout_config.leaf_measure.measure(
                        leaf,
                        &mut self.leaf_data,
                        known_dimensions,
                        available_space,
                    )
                },
            )
        }
        else {
            // we need to explicitely drop the node.
            //
            // Node is generic over L, so we don't know if it has a Drop impl, thus it can
            // potentially live until the end of the scope. But we its lifetime to stop here
            // so we can pass the tree to the recursive call.
            let display = node.style.display;
            drop(node);

            match display {
                taffy::Display::Block => taffy::compute_block_layout(self, node_id, inputs),
                taffy::Display::Flex => taffy::compute_flexbox_layout(self, node_id, inputs),
                taffy::Display::Grid => taffy::compute_grid_layout(self, node_id, inputs),
                taffy::Display::None => taffy::compute_hidden_layout(self, node_id),
            }
        }
    }
}

impl<'w, 's, L> LayoutPartialTree for Tree<'w, 's, L>
where
    L: LeafMeasure,
{
    type CoreContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    type CustomIdent = String;

    fn get_core_container_style(&self, node_id: taffy::NodeId) -> Self::CoreContainerStyle<'_> {
        self.get_style_for_node(node_id)
    }

    fn set_unrounded_layout(&mut self, node_id: taffy::NodeId, layout: &taffy::Layout) {
        let mut unrounded_layout = self
            .nodes
            .get_mut(node_id_to_entity(node_id))
            .unwrap()
            .unrounded_layout;

        if &unrounded_layout.0 != layout {
            unrounded_layout.0 = *layout;
        }
    }

    fn compute_child_layout(
        &mut self,
        node_id: taffy::NodeId,
        inputs: taffy::LayoutInput,
    ) -> taffy::LayoutOutput {
        taffy::compute_cached_layout(self, node_id, inputs, |tree, node_id, inputs| {
            tree.compute_uncached_child_layout(node_id, inputs)
        })
        //self.compute_uncached_child_layout(node_id, inputs)
    }

    fn resolve_calc_value(&self, val: *const (), basis: f32) -> f32 {
        let _ = (val, basis);
        panic!("no calc values");
    }
}

impl<'w, 's, L> TraversePartialTree for Tree<'w, 's, L>
where
    L: LeafMeasure,
{
    type ChildIter<'a>
        = ChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        ChildIter {
            inner: self
                .nodes
                .get(node_id_to_entity(node_id))
                .ok()
                .and_then(|node| node.children)
                .map_or([].iter(), |children| children.iter()),
        }
    }

    fn child_count(&self, node_id: NodeId) -> usize {
        self.nodes
            .get(node_id_to_entity(node_id))
            .ok()
            .and_then(|node| node.children)
            .map_or(0, |children| children.len())
    }

    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        let node = self.nodes.get(node_id_to_entity(node_id)).unwrap();
        let children = node.children.unwrap();
        entity_to_node_id(children[index])
    }
}

impl<'w, 's, L> taffy::TraverseTree for Tree<'w, 's, L> where L: LeafMeasure {}

impl<'w, 's, L> CacheTree for Tree<'w, 's, L>
where
    L: LeafMeasure,
{
    fn cache_get(
        &self,
        node_id: NodeId,
        known_dimensions: taffy::Size<Option<f32>>,
        available_space: taffy::Size<taffy::AvailableSpace>,
        run_mode: taffy::RunMode,
    ) -> Option<taffy::LayoutOutput> {
        self.nodes
            .get(node_id_to_entity(node_id))
            .ok()?
            .cache
            .0
            .get(known_dimensions, available_space, run_mode)
    }

    fn cache_store(
        &mut self,
        node_id: NodeId,
        known_dimensions: taffy::Size<Option<f32>>,
        available_space: taffy::Size<taffy::AvailableSpace>,
        run_mode: taffy::RunMode,
        layout_output: taffy::LayoutOutput,
    ) {
        if let Ok(mut node) = self.nodes.get_mut(node_id_to_entity(node_id)) {
            node.cache
                .0
                .store(known_dimensions, available_space, run_mode, layout_output);
        }
    }

    fn cache_clear(&mut self, node_id: NodeId) {
        if let Ok(mut node) = self.nodes.get_mut(node_id_to_entity(node_id)) {
            node.cache.0.clear();
        }
    }
}

impl<'w, 's, L> taffy::LayoutBlockContainer for Tree<'w, 's, L>
where
    L: LeafMeasure,
{
    type BlockContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    type BlockItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: NodeId) -> Self::BlockContainerStyle<'_> {
        self.get_style_for_node(node_id)
    }

    fn get_block_child_style(&self, child_node_id: NodeId) -> Self::BlockItemStyle<'_> {
        self.get_style_for_node(child_node_id)
    }
}

impl<'w, 's, L> taffy::LayoutFlexboxContainer for Tree<'w, 's, L>
where
    L: LeafMeasure,
{
    type FlexboxContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    type FlexboxItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node_id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        self.get_style_for_node(node_id)
    }

    fn get_flexbox_child_style(&self, child_node_id: NodeId) -> Self::FlexboxItemStyle<'_> {
        self.get_style_for_node(child_node_id)
    }
}

impl<'w, 's, L> taffy::LayoutGridContainer for Tree<'w, 's, L>
where
    L: LeafMeasure,
{
    type GridContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    type GridItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: NodeId) -> Self::GridContainerStyle<'_> {
        self.get_style_for_node(node_id)
    }

    fn get_grid_child_style(&self, child_node_id: NodeId) -> Self::GridItemStyle<'_> {
        self.get_style_for_node(child_node_id)
    }
}

impl<'w, 's, L> taffy::RoundTree for Tree<'w, 's, L>
where
    L: LeafMeasure,
{
    fn get_unrounded_layout(&self, node_id: NodeId) -> taffy::Layout {
        self.nodes
            .get(node_id_to_entity(node_id))
            .unwrap()
            .unrounded_layout
            .0
    }

    fn set_final_layout(&mut self, node_id: NodeId, layout: &taffy::Layout) {
        let entity = node_id_to_entity(node_id);
        let rounded_layout = self.nodes.get_mut(entity).unwrap().rounded_layout;

        if layout.size == taffy::Size::ZERO {
            if rounded_layout.is_some() {
                self.commands.entity(entity).try_remove::<RoundedLayout>();
            }
        }
        else {
            if let Some(mut rounded_layout) = rounded_layout {
                if &rounded_layout.0 != layout {
                    tracing::trace!(?entity, ?layout, "final layout");
                    rounded_layout.0 = *layout;
                }
            }
            else {
                tracing::trace!(?entity, ?layout, "final layout");
                self.commands.entity(entity).insert(RoundedLayout(*layout));
            }
        }
    }
}

impl<'w, 's, L> taffy::PrintTree for Tree<'w, 's, L>
where
    L: LeafMeasure,
{
    fn get_debug_label(&self, node_id: NodeId) -> &'static str {
        self.nodes
            .get(node_id_to_entity(node_id))
            .unwrap()
            .debug_label
            .map(|label| label.0)
            .unwrap_or_default()
    }

    fn get_final_layout(&self, node_id: NodeId) -> taffy::Layout {
        self.nodes
            .get(node_id_to_entity(node_id))
            .unwrap()
            .rounded_layout
            .map(|rounded_layout| rounded_layout.0)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct ChildIter<'a> {
    inner: std::slice::Iter<'a, Entity>,
}

impl<'a> Iterator for ChildIter<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().copied().map(entity_to_node_id)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

mod style_not_send_sync_patch {
    //! Taffy 0.8 made this not Send/Sync anymore because it contains a raw
    //! pointer in `Dimension`.
    //!
    //! bevy_ui works around this by just making this Send/Sync but disabling
    //! the `calc` feature. Apparently this is safe to do.
    //!
    //! - [Taffy Issue #823](https://github.com/DioxusLabs/taffy/issues/823)
    //! - [Bevy PR](https://github.com/bevyengine/bevy/pull/21672)

    use taffy::CheapCloneStr;

    #[derive(Clone, Debug, Default, PartialEq)]
    pub(super) struct Style<S = String>(pub taffy::Style<S>)
    where
        S: CheapCloneStr;

    /// # Safety
    /// Taffy Tree becomes thread unsafe when you use calc(), which we do not
    /// implement
    #[expect(
        unsafe_code,
        reason = "This wrapper is safe while the calc feature is disabled."
    )]
    unsafe impl<S> Send for Style<S> where S: CheapCloneStr {}

    /// # Safety
    /// Taffy Tree becomes thread unsafe when you use calc(), which we do not
    /// implement
    #[expect(
        unsafe_code,
        reason = "This wrapper is safe while the calc feature is disabled."
    )]
    unsafe impl<S> Sync for Style<S> where S: CheapCloneStr {}
}
