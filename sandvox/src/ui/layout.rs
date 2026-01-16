use std::ops::{
    Deref,
    DerefMut,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    hierarchy::Children,
    query::{
        QueryData,
        With,
    },
    system::{
        Query,
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

use crate::ui::{
    UiRoot,
    UiSurface,
};

/// System that computes the layout of all UI treees
pub(super) fn layout_trees(mut tree: Tree, roots: Query<(Entity, &UiSurface), With<UiRoot>>) {
    for (entity, surface) in roots.iter() {
        tree.compute_root_layout(entity, surface.size);
    }
}

#[derive(Clone, Debug, Default, Component)]
pub struct Style(style_not_send_sync_patch::Style);

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
struct Cache(taffy::Cache);

#[derive(Clone, Debug, Default, Component)]
struct UnroundedLayout(taffy::Layout);

#[derive(Clone, Debug, Default, Component, derive_more::Deref)]
pub struct RoundedLayout(taffy::Layout);

impl RoundedLayout {
    pub fn is_visible(&self) -> bool {
        self.0.size != taffy::Size::ZERO
    }
}

#[derive(Clone, Copy, Debug, Component)]
struct DebugLabel(&'static str);

#[derive(Debug, Component)]
pub enum LeafMeasure {
    Pending {
        known_dimensions: Size<Option<f32>>,
        available_space: Size<AvailableSpace>,
    },
    Response {
        measured_size: Size<f32>,
    },
}

impl Default for LeafMeasure {
    fn default() -> Self {
        Self::Pending {
            known_dimensions: Default::default(),
            available_space: Size {
                width: AvailableSpace::MinContent,
                height: AvailableSpace::MinContent,
            },
        }
    }
}

impl LeafMeasure {
    pub fn measured_size(&self) -> Option<Size<f32>> {
        match self {
            LeafMeasure::Pending { .. } => None,
            LeafMeasure::Response { measured_size } => Some(*measured_size),
        }
    }

    pub fn respond_with(
        &mut self,
        mut measure_function: impl FnMut(Size<Option<f32>>, Size<AvailableSpace>) -> Option<Size<f32>>,
    ) {
        match self {
            LeafMeasure::Pending {
                known_dimensions,
                available_space,
            } => {
                let measured_size = measure_function(*known_dimensions, *available_space)
                    .unwrap_or_else(|| {
                        Size {
                            width: known_dimensions.width.unwrap_or_default(),
                            height: known_dimensions.height.unwrap_or_default(),
                        }
                    });
                *self = LeafMeasure::Response { measured_size };
            }
            LeafMeasure::Response { .. } => {}
        }
    }
}

#[derive(Debug, QueryData)]
#[query_data(mutable)]
struct Node {
    style: &'static Style,
    unrounded_layout: &'static mut UnroundedLayout,
    rounded_layout: &'static mut RoundedLayout,
    cache: &'static mut Cache,
    leaf_measure: Option<&'static mut LeafMeasure>,
    debug_label: Option<&'static DebugLabel>,
}

#[inline(always)]
fn node_id_to_entity(node_id: NodeId) -> Entity {
    Entity::from_bits(node_id.into())
}

#[inline(always)]
fn entity_to_node_id(entity: Entity) -> NodeId {
    entity.to_bits().into()
}

#[derive(Debug, SystemParam)]
pub(super) struct Tree<'w, 's> {
    nodes: Query<'w, 's, Node>,
    children: Query<'w, 's, &'static Children>,
}

impl<'w, 's> Tree<'w, 's> {
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
}

impl<'w, 's> LayoutPartialTree for Tree<'w, 's> {
    type CoreContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    type CustomIdent = String;

    fn get_core_container_style(&self, node_id: taffy::NodeId) -> Self::CoreContainerStyle<'_> {
        self.get_style_for_node(node_id)
    }

    fn set_unrounded_layout(&mut self, node_id: taffy::NodeId, layout: &taffy::Layout) {
        self.nodes
            .get_mut(node_id_to_entity(node_id))
            .unwrap()
            .unrounded_layout
            .0 = *layout;
    }

    fn compute_child_layout(
        &mut self,
        node_id: taffy::NodeId,
        inputs: taffy::LayoutInput,
    ) -> taffy::LayoutOutput {
        taffy::compute_cached_layout(self, node_id, inputs, |tree, node_id, inputs| {
            let node = tree.nodes.get_mut(node_id_to_entity(node_id)).unwrap();

            if let Some(mut leaf_measure) = node.leaf_measure {
                taffy::compute_leaf_layout(
                    inputs,
                    &**node.style,
                    |_calc_ptr, _parent_size| 0.0,
                    |known_dimensions, available_space| {
                        let measured_size = leaf_measure.measured_size();
                        *leaf_measure = LeafMeasure::Pending {
                            known_dimensions,
                            available_space,
                        };
                        measured_size.unwrap_or_default()
                    },
                )
            }
            else {
                match node.style.display {
                    taffy::Display::Block => taffy::compute_block_layout(tree, node_id, inputs),
                    taffy::Display::Flex => taffy::compute_flexbox_layout(tree, node_id, inputs),
                    taffy::Display::Grid => taffy::compute_grid_layout(tree, node_id, inputs),
                    taffy::Display::None => taffy::compute_hidden_layout(tree, node_id),
                }
            }
        })
    }
}

impl<'w, 's> TraversePartialTree for Tree<'w, 's> {
    type ChildIter<'a>
        = ChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        ChildIter {
            inner: self
                .children
                .get(node_id_to_entity(node_id))
                .unwrap()
                .iter(),
        }
    }

    fn child_count(&self, node_id: NodeId) -> usize {
        self.children.get(node_id_to_entity(node_id)).unwrap().len()
    }

    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        entity_to_node_id(self.children.get(node_id_to_entity(node_id)).unwrap()[index])
    }
}

impl<'w, 's> taffy::TraverseTree for Tree<'w, 's> {}

impl<'w, 's> CacheTree for Tree<'w, 's> {
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

impl<'w, 's> taffy::LayoutBlockContainer for Tree<'w, 's> {
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

impl<'w, 's> taffy::LayoutFlexboxContainer for Tree<'w, 's> {
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

impl<'w, 's> taffy::LayoutGridContainer for Tree<'w, 's> {
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

impl<'w, 's> taffy::RoundTree for Tree<'w, 's> {
    fn get_unrounded_layout(&self, node_id: NodeId) -> taffy::Layout {
        self.nodes
            .get(node_id_to_entity(node_id))
            .unwrap()
            .unrounded_layout
            .0
    }

    fn set_final_layout(&mut self, node_id: NodeId, layout: &taffy::Layout) {
        self.nodes
            .get_mut(node_id_to_entity(node_id))
            .unwrap()
            .rounded_layout
            .0 = *layout;
    }
}

impl<'w, 's> taffy::PrintTree for Tree<'w, 's> {
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
            .0
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
