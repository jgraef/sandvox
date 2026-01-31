use std::ops::{
    Deref,
    DerefMut,
};

use bevy_ecs::{
    change_detection::DetectChangesMut,
    component::Component,
    entity::{
        Entity,
        EntityHashSet,
    },
    hierarchy::{
        ChildOf,
        Children,
    },
    name::NameOrEntity,
    query::{
        Changed,
        Or,
        QueryData,
        With,
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
        Root,
        UiSystems,
        view::View,
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
            purge_invalid_cache_entries,
            (calculate_tree_layouts::<L>, finalize_tree_layouts::<L>)
                .chain()
                .run_if(any_match_filter::<Or<(Changed<LayoutCache>, Changed<View>)>>)
                .after(purge_invalid_cache_entries)
                .after(UiSystems::Layout)
                .before(UiSystems::Render),
            request_redraw
                .before(UiSystems::Render)
                .after(finalize_tree_layouts::<L>),
        ),
    );
}

fn request_redraw(nodes: Populated<&Root, Changed<FinalLayout>>, mut views: Populated<&mut View>) {
    for root in nodes {
        let mut view = views.get_mut(root.root).unwrap();
        view.render = true;
    }
}

/// System that computes the (unrounded) layout of all UI trees
///
/// # TODO
///
/// make this run only if something changes. Preferably only compute subtrees
/// if they changed. taffy caches them, but we do know when stuff changes.
/// Could work like the transform hierarchy.
///
/// This might help too:
///
/// ```no_run
/// .run_if(any_match_filter::<Or<(Changed<Style>, Changed<LeafMeasure>)>>)
/// ```
#[profiling::function]
fn calculate_tree_layouts<L>(mut tree: TreeInner<L>, views: Populated<(Entity, &View)>)
where
    L: LeafMeasure,
{
    for (entity, view) in views.iter() {
        let mut tree = Tree {
            inner: &mut tree,
            root: Root { root: entity },
        };
        tree.compute_layout(view.size.cast());
    }
}

/// System that computes the rounded layouts
#[profiling::function]
fn finalize_tree_layouts<L>(mut tree: TreeInner<L>, views: Populated<Entity, With<View>>)
where
    L: LeafMeasure,
{
    for entity in views.iter() {
        let mut tree = Tree {
            inner: &mut tree,
            root: Root { root: entity },
        };
        tree.finalize_layout();
    }
}

// not needed anymore
/*#[profiling::function]
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
}*/

/// This purges all cache entries that have become invalid
///
/// # TODO
///
/// I'm not sure if this is correct
#[profiling::function]
fn purge_invalid_cache_entries(
    mut params: ParamSet<(
        Populated<Entity, Or<(Changed<LayoutCache>, Changed<ChildOf>, Changed<Children>)>>,
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
pub struct FinalLayout {
    #[deref]
    layout: taffy::Layout,
    pub depth: u32,
}

#[inline]
fn node_id_to_entity(node_id: NodeId) -> Entity {
    Entity::from_bits(node_id.into())
}

#[inline]
fn entity_to_node_id(entity: Entity) -> NodeId {
    entity.to_bits().into()
}

#[derive(Debug, Resource)]
pub(super) struct LayoutConfig<L> {
    pub leaf_measure: L,
}

struct Tree<'t, 'w, 's, L>
where
    L: LeafMeasure,
{
    inner: &'t mut TreeInner<'w, 's, L>,
    root: Root,
}

#[derive(SystemParam)]
struct TreeInner<'w, 's, L>
where
    L: LeafMeasure,
{
    styles: Query<'w, 's, &'static Style>,
    unrounded_layouts: Query<'w, 's, &'static mut UnroundedLayout>,
    final_layouts: Query<'w, 's, &'static mut FinalLayout>,
    roots: Query<'w, 's, &'static mut Root>,
    cache: Query<'w, 's, &'static mut LayoutCache>,
    children: Query<'w, 's, &'static Children>,
    leafs: Query<'w, 's, <L as LeafMeasure>::Node>,

    /// for displaying better error messages
    names: Query<'w, 's, NameOrEntity>,

    /// data required by the leaf-measurement function
    leaf_data: StaticSystemParam<'w, 's, <L as LeafMeasure>::Data>,

    layout_config: Res<'w, LayoutConfig<L>>,

    commands: Commands<'w, 's>,
}

impl<'t, 'w, 's, L> Tree<'t, 'w, 's, L>
where
    L: LeafMeasure,
{
    fn compute_layout(&mut self, view_size: Vector2<f32>) {
        let available_size = Size {
            width: AvailableSpace::Definite(view_size.x),
            height: AvailableSpace::Definite(view_size.y),
        };

        let root = entity_to_node_id(self.root.root);
        taffy::compute_root_layout(self, root, available_size);
    }

    fn finalize_layout(&mut self) {
        let root = entity_to_node_id(self.root.root);
        taffy::round_layout(self, root);

        // the `order` field in `taffy::Layout` is not global for the whole tree, so we
        // need to assign our own order values
        //
        // https://github.com/DioxusLabs/taffy/issues/226

        fn toposort(
            entity: Entity,
            depth: u32,
            mut final_layouts: &mut Query<&mut FinalLayout>,
            children_query: &Query<&Children>,
        ) {
            if let Ok(mut final_layout) = final_layouts.get_mut(entity) {
                if final_layout.depth != depth {
                    final_layout.depth = depth;
                }

                if let Ok(children) = children_query.get(entity) {
                    for child in children.iter() {
                        toposort(*child, depth + 1, &mut final_layouts, children_query);
                    }
                }
            }
        }

        toposort(
            self.root.root,
            0,
            &mut self.inner.final_layouts,
            &self.inner.children,
        );
    }

    fn get_style_for_node(&self, node_id: NodeId) -> &taffy::Style {
        let entity = node_id_to_entity(node_id);

        /*let style = self.inner.styles.get(entity).unwrap_or_else(|error| {
            let name = self.inner.names.get(entity).unwrap();
            panic!("Can't get style for UI node `{name}`: {error}");
        });
        &**style
        */

        self.inner.styles.get(entity).map_or_else(
            |_error| get_default_style_for_node(&self.root, entity),
            |style| &**style,
        )
    }

    fn compute_uncached_child_layout(
        &mut self,
        node_id: taffy::NodeId,
        inputs: taffy::LayoutInput,
    ) -> taffy::LayoutOutput {
        let entity = node_id_to_entity(node_id);
        tracing::trace!(?entity, "computing tree layout for node");

        if let Ok(mut leaf) = self.inner.leafs.get_mut(entity) {
            let style = self.inner.styles.get(entity).map_or_else(
                |_error| get_default_style_for_node(&self.root, entity),
                |style| &**style,
            );

            taffy::compute_leaf_layout(
                inputs,
                style,
                |_calc_ptr, _parent_size| 0.0,
                |known_dimensions, available_space| {
                    self.inner.layout_config.leaf_measure.measure(
                        &mut leaf,
                        &mut self.inner.leaf_data,
                        known_dimensions,
                        available_space,
                    )
                },
            )
        }
        else {
            let display = self
                .inner
                .styles
                .get(entity)
                .map_or(const { taffy::Display::DEFAULT }, |style| style.display);

            match display {
                taffy::Display::Block => taffy::compute_block_layout(self, node_id, inputs),
                taffy::Display::Flex => taffy::compute_flexbox_layout(self, node_id, inputs),
                taffy::Display::Grid => taffy::compute_grid_layout(self, node_id, inputs),
                taffy::Display::None => taffy::compute_hidden_layout(self, node_id),
            }
        }
    }
}

impl<'t, 'w, 's, L> LayoutPartialTree for Tree<'t, 'w, 's, L>
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
        let entity = node_id_to_entity(node_id);

        if let Ok(mut unrounded_layout) = self.inner.unrounded_layouts.get_mut(entity) {
            // only modify if it really changed
            if &unrounded_layout.0 != layout {
                unrounded_layout.0 = *layout;
            }
        }
        else {
            self.inner
                .commands
                .entity(entity)
                .insert(UnroundedLayout(*layout));
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

impl<'t, 'w, 's, L> TraversePartialTree for Tree<'t, 'w, 's, L>
where
    L: LeafMeasure,
{
    type ChildIter<'a>
        = ChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        let entity = node_id_to_entity(node_id);
        let inner = self
            .inner
            .children
            .get(entity)
            .ok()
            .map_or([].iter(), |children| children.iter());

        ChildIter { inner }
    }

    fn child_count(&self, node_id: NodeId) -> usize {
        let entity = node_id_to_entity(node_id);
        self.inner
            .children
            .get(entity)
            .map_or(0, |children| children.len())
    }

    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        let entity = node_id_to_entity(node_id);

        let children = self.inner.children.get(entity).unwrap_or_else(|_| {
            let name = self.inner.names.get(entity).unwrap();
            panic!("Node `{name}` has no children");
        });

        let child = children.get(index).unwrap_or_else(|| {
            let name = self.inner.names.get(entity).unwrap();
            panic!("Node `{name}` has no child with index `{index}`");
        });

        entity_to_node_id(*child)
    }
}

impl<'t, 'w, 's, L> taffy::TraverseTree for Tree<'t, 'w, 's, L> where L: LeafMeasure {}

impl<'t, 'w, 's, L> CacheTree for Tree<'t, 'w, 's, L>
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
        let entity = node_id_to_entity(node_id);

        self.inner
            .cache
            .get(entity)
            .ok()?
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
        let entity = node_id_to_entity(node_id);

        if let Ok(mut cache) = self.inner.cache.get_mut(entity) {
            cache
                .0
                .store(known_dimensions, available_space, run_mode, layout_output);
        }
        else {
            let mut cache = LayoutCache::default();

            cache
                .0
                .store(known_dimensions, available_space, run_mode, layout_output);

            self.inner.commands.entity(entity).insert(cache);
        }
    }

    fn cache_clear(&mut self, node_id: NodeId) {
        let entity = node_id_to_entity(node_id);
        if let Ok(mut cache) = self.inner.cache.get_mut(entity) {
            cache.0.clear();
        }
    }
}

impl<'t, 'w, 's, L> taffy::LayoutBlockContainer for Tree<'t, 'w, 's, L>
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

impl<'t, 'w, 's, L> taffy::LayoutFlexboxContainer for Tree<'t, 'w, 's, L>
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

impl<'t, 'w, 's, L> taffy::LayoutGridContainer for Tree<'t, 'w, 's, L>
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

impl<'t, 'w, 's, L> taffy::RoundTree for Tree<'t, 'w, 's, L>
where
    L: LeafMeasure,
{
    fn get_unrounded_layout(&self, node_id: NodeId) -> taffy::Layout {
        let entity = node_id_to_entity(node_id);

        let unrounded_layout = self
            .inner
            .unrounded_layouts
            .get(entity)
            .unwrap_or_else(|_| {
                let name = self.inner.names.get(entity).unwrap();
                panic!("Node `{name}` has no unrounded layout");
            });

        unrounded_layout.0
    }

    fn set_final_layout(&mut self, node_id: NodeId, layout: &taffy::Layout) {
        let entity = node_id_to_entity(node_id);

        if let Ok(mut rounded_layout) = self.inner.final_layouts.get_mut(entity) {
            if layout.size == taffy::Size::ZERO {
                // remove final layout

                self.inner
                    .commands
                    .entity(entity)
                    .try_remove::<FinalLayout>();
            }
            else if &rounded_layout.layout != layout {
                // update final layout

                tracing::trace!(?entity, ?layout, "final layout");
                rounded_layout.layout = *layout;
            }
        }
        else if layout.size != taffy::Size::ZERO {
            // insert final layout

            tracing::trace!(?entity, ?layout, "final layout");
            self.inner.commands.entity(entity).insert(FinalLayout {
                layout: *layout,
                depth: 0,
            });
        }

        // update/insert root
        if let Ok(mut root) = self.inner.roots.get_mut(entity) {
            root.set_if_neq(self.root);
        }
        else {
            self.inner.commands.entity(entity).insert(self.root);
        }
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

fn get_default_style_for_node(root: &Root, entity: Entity) -> &'static taffy::Style {
    if entity == root.root {
        const ROOT_DEFAULT: taffy::Style = const {
            let mut root_default = taffy::Style::DEFAULT;

            let full = taffy::Dimension::percent(1.0);
            root_default.size = taffy::Size {
                width: full,
                height: full,
            };
            root_default.display = taffy::Display::None;

            root_default
        };

        const { &ROOT_DEFAULT }
    }
    else {
        const { &taffy::Style::DEFAULT }
    }
}
