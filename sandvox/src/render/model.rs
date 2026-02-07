use std::{
    any::type_name,
    collections::{
        HashMap,
        hash_map,
    },
    marker::PhantomData,
    path::Path,
};

use bevy_ecs::{
    entity::Entity,
    hierarchy::ChildOf,
    name::Name,
    relationship::RelatedSpawnerCommands,
    system::{
        Commands,
        EntityCommands,
        Res,
        SystemParam,
    },
};
use bytemuck::AnyBitPattern;
use color_eyre::eyre::{
    Error,
    bail,
    eyre,
};
use nalgebra::{
    Isometry3,
    Point2,
    Point3,
    Quaternion,
    Translation3,
    UnitQuaternion,
    Vector3,
};

use crate::{
    ecs::transform::LocalTransform,
    render::{
        animation::{
            Channel,
            KeyFrame,
        },
        mesh::{
            Mesh,
            MeshBufferSpan,
            MeshPipelineLayout,
            Vertex,
        },
    },
    wgpu::WgpuContext,
};

#[derive(derive_more::Debug, SystemParam)]
pub struct ModelLoader<'w, 's> {
    wgpu: Res<'w, WgpuContext>,
    mesh_layout: Res<'w, MeshPipelineLayout>,

    #[debug(skip)]
    commands: Commands<'w, 's>,
}

impl<'w, 's> ModelLoader<'w, 's> {
    pub fn load_scene(&mut self, path: impl AsRef<Path>) -> Result<EntityCommands<'_>, Error> {
        let gltf = gltf::Gltf::open(path)?;

        let mut importer = ModelImporter::new(&gltf)?;
        let mut scene_entity = importer.import_default_scene(&mut self.commands)?;

        // reborrow commands from the scene entity
        let commands = scene_entity.commands_mut();

        // import meshes. this actually creates the mesh buffers and attaches `Mesh`
        // components
        importer.import_meshes(&self.wgpu, &self.mesh_layout, commands)?;

        // import animations
        importer.import_animations(commands)?;

        Ok(scene_entity)
    }
}

#[derive(derive_more::Debug)]
pub struct ModelImporter<'a> {
    #[debug(skip)]
    gltf: &'a gltf::Gltf,

    #[debug(skip)]
    load_meshes: Vec<(Entity, gltf::Mesh<'a>)>,

    label: Option<&'a str>,

    node_to_entity: HashMap<usize, Entity>,
}

impl<'a> ModelImporter<'a> {
    pub fn new(gltf: &'a gltf::Gltf) -> Result<Self, Error> {
        Ok(Self {
            gltf,
            load_meshes: vec![],
            label: None,
            node_to_entity: HashMap::new(),
        })
    }

    pub fn set_label(&mut self, label: &'a str) {
        self.label = Some(label);
    }

    /// Imports a scene, i.e. all nodes in that scene
    ///
    /// An entity is created as a parent for all nodes in the scene. This entity
    /// is returned.
    pub fn import_scene<'c>(
        &mut self,
        scene: &gltf::Scene<'a>,
        commands: &'c mut Commands,
    ) -> Result<EntityCommands<'c>, Error> {
        let mut scene_entity = commands.spawn_empty();

        if let Some(name) = scene.name() {
            scene_entity.insert(Name::new(name.to_owned()));
        }

        // the caller should add a LocalTransform, but stuff doesn't work without one,
        // so we'll add the identity in case the caller forgets
        scene_entity.insert(LocalTransform::identity());

        let scene_entity_id = scene_entity.id();
        let mut child_spawner =
            RelatedSpawnerCommands::new(scene_entity.commands(), scene_entity_id);

        for node in scene.nodes() {
            self.import_node(&node, &mut child_spawner)?;
        }

        Ok(scene_entity)
    }

    /// Imports the default scene.
    ///
    /// See [`import_scene`].
    pub fn import_default_scene<'c>(
        &mut self,
        commands: &'c mut Commands,
    ) -> Result<EntityCommands<'c>, Error> {
        let scene = self
            .gltf
            .default_scene()
            .ok_or_else(|| eyre!("No default scene"))?;

        if self.label.is_none()
            && let Some(name) = scene.name()
        {
            self.label = Some(name);
        }

        self.import_scene(&scene, commands)
    }

    /// Imports a node.
    ///
    /// This also imports all children of that node.
    ///
    /// The entity that is created for this node is returned.
    ///
    /// Note that this will keep track of which entities have which meshes, but
    /// will not load them yet.
    pub fn import_node<'c>(
        &mut self,
        node: &gltf::Node<'a>,
        commands: &'c mut RelatedSpawnerCommands<ChildOf>,
    ) -> Result<EntityCommands<'c>, Error> {
        let mut node_entity = commands.spawn_empty();
        let node_entity_id = node_entity.id();

        // add name
        if let Some(name) = node.name() {
            node_entity.insert(Name::new(name.to_owned()));
        }

        // add transform
        node_entity.insert(convert_transform(node.transform()));

        // we need to remember the mapping of node ID to entity ID if we want to add
        // animations later
        self.node_to_entity.insert(node.index(), node_entity_id);

        // remember for later to add this mesh to this entity
        if let Some(mesh) = node.mesh() {
            self.load_meshes.push((node_entity_id, mesh));
        }

        // import children
        let mut child_spawner = RelatedSpawnerCommands::new(node_entity.commands(), node_entity_id);
        for child in node.children() {
            self.import_node(&child, &mut child_spawner)?;
        }

        Ok(node_entity)
    }

    /// Import meshes and attach them to entities.
    ///
    /// This loads all meshes for nodes that have been imported. All meshes are
    /// stored in a combined index and vertex buffer, which is shared between
    /// all entities. This attaches [`Mesh`] components to the entities.
    pub fn import_meshes(
        &mut self,
        wgpu: &WgpuContext,
        mesh_layout: &MeshPipelineLayout,
        commands: &mut Commands,
    ) -> Result<(), Error> {
        if self.load_meshes.is_empty() {
            // early return.
            //
            // not only does this prevent us from trying to create an empty mesh buffer.
            // this will also avoid returning an error if there is no blob in this file
            return Ok(());
        }

        let mut loaded_meshes: HashMap<usize, Option<(MeshBufferSpan, gltf::Primitive<'_>)>> =
            HashMap::new();

        // initial pass to just reserve space for all buffers and get the total size

        let mut vertex_buffer_offset = 0;
        let mut index_buffer_offset = 0;

        for (_entity, mesh) in &self.load_meshes {
            match loaded_meshes.entry(mesh.index()) {
                hash_map::Entry::Occupied(_occupied_entry) => {}
                hash_map::Entry::Vacant(vacant_entry) => {
                    if let Some(primitive) = get_first_tri_primitive(&mesh) {
                        let num_indices = primitive
                            .indices()
                            .unwrap_or_else(|| todo!("Mesh without index buffer"))
                            .count()
                            .try_into()
                            .unwrap();
                        let num_vertices = get_num_vertices(&primitive)?;

                        let span = MeshBufferSpan {
                            vertex_buffer_offset,
                            num_vertices,
                            index_buffer_offset,
                            num_indices,
                        };

                        vertex_buffer_offset += num_vertices;
                        index_buffer_offset += num_indices;

                        vacant_entry.insert(Some((span, primitive)));
                    }
                    else {
                        vacant_entry.insert(None);
                    }
                }
            }
        }

        if vertex_buffer_offset == 0 || index_buffer_offset == 0 || loaded_meshes.is_empty() {
            // all meshes are empty

            // if either is true, all must be true
            assert!(
                vertex_buffer_offset == 0 && index_buffer_offset == 0 && loaded_meshes.is_empty()
            );

            return Ok(());
        }

        // allocate buffers
        //
        // todo: since they're just storage buffers we could also merge them

        let vertex_buffer = wgpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: self.label,
            size: (u64::from(vertex_buffer_offset) * u64::try_from(size_of::<Vertex>()).unwrap()),
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        });

        let index_buffer = wgpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: self.label,
            size: (u64::from(index_buffer_offset) * u64::try_from(size_of::<u32>()).unwrap()),
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        });

        let bind_group = wgpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: self.label,
            layout: &mesh_layout.mesh_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: vertex_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: index_buffer.as_entire_binding(),
                },
            ],
        });

        {
            // fill buffers

            let blob = self
                .gltf
                .blob
                .as_ref()
                .ok_or_else(|| eyre!("GLTF file without binary blob"))?;

            let mut vertex_buffer_view = vertex_buffer.get_mapped_range_mut(..);
            let vertex_buffer_view =
                bytemuck::cast_slice_mut::<u8, Vertex>(&mut *vertex_buffer_view);

            let mut index_buffer_view = index_buffer.get_mapped_range_mut(..);
            let index_buffer_view = bytemuck::cast_slice_mut::<u8, u32>(&mut *index_buffer_view);

            for (span, primitive) in loaded_meshes
                .iter()
                .filter_map(|(_mesh_id, entry)| entry.as_ref())
            {
                fill_index_buffer(primitive, blob, index_buffer_view, span)?;
                fill_vertex_buffer(primitive, blob, vertex_buffer_view, span)?;
            }
        }

        // unmap buffers
        vertex_buffer.unmap();
        index_buffer.unmap();

        // insert mesh components for each entity
        for (entity, mesh) in self.load_meshes.drain(..) {
            if let Some((span, _pimitive)) = loaded_meshes
                .get(&mesh.index())
                .expect("missing load_meshes entry")
            {
                commands.entity(entity).insert(Mesh {
                    vertex_buffer: vertex_buffer.clone(),
                    index_buffer: index_buffer.clone(),
                    bind_group: bind_group.clone(),
                    span: *span,
                });
            }
        }

        Ok(())
    }

    pub fn import_animations(&mut self, commands: &mut Commands) -> Result<(), Error> {
        // get the blob. return an empty one if necessary
        let blob = self.gltf.blob.as_deref().unwrap_or_default();

        for animation in self.gltf.animations() {
            tracing::debug!(name = ?animation.name(), "animation");

            let mut channels = Vec::with_capacity(animation.channels().count());

            for channel in animation.channels() {
                let target = channel.target();

                let target_entity_id = *self
                    .node_to_entity
                    .get(&target.node().index())
                    .ok_or_else(|| {
                        eyre!(
                            "Animation for node that was not imported: #{}",
                            target.node().index()
                        )
                    })?;

                let target_property = target.property();

                tracing::debug!(index = ?channel.index(), ?target_entity_id, ?target_property, "channel");

                let sampler = channel.sampler();

                let input = sampler.input();
                let output = sampler.output();
                if input.count() != output.count() {
                    bail!(
                        "Animation #{} channel #{} input frame count {} doesn't match output frame count {}",
                        animation.index(),
                        channel.index(),
                        input.count(),
                        output.count()
                    );
                }
                let num_frames = input.count();

                tracing::debug!(animation = ?animation.name(), data_type = ?input.data_type(), dimensions = ?input.dimensions(), "input");
                tracing::debug!(animation = ?animation.name(), data_type = ?output.data_type(), dimensions = ?output.dimensions(), "output");

                let channel = match target_property {
                    gltf::animation::Property::Translation => {
                        import_animation_channel::<[f32; 3], _>(
                            blob,
                            &input,
                            &output,
                            num_frames,
                            |data| Translation3::from(Vector3::from(data)),
                            |key_frames| {
                                Channel::Translation {
                                    target: target_entity_id,
                                    key_frames,
                                }
                            },
                        )?
                    }
                    gltf::animation::Property::Rotation => {
                        import_animation_channel::<[f32; 4], _>(
                            blob,
                            &input,
                            &output,
                            num_frames,
                            |data| UnitQuaternion::new_unchecked(data.into()),
                            |key_frames| {
                                Channel::Rotation {
                                    target: target_entity_id,
                                    key_frames,
                                }
                            },
                        )?
                    }
                    gltf::animation::Property::Scale => {
                        tracing::warn!("Ignoring scale animation");
                        continue;
                    }
                    gltf::animation::Property::MorphTargetWeights => {
                        todo!("animate MorphTargetWeights");
                    }
                };

                channels.push(channel);
            }
        }

        todo!();
    }
}

fn get_first_tri_primitive<'a>(mesh: &gltf::Mesh<'a>) -> Option<gltf::Primitive<'a>> {
    mesh.primitives()
        .find(|primitive| primitive.mode() == gltf::mesh::Mode::Triangles)
}

fn get_num_vertices(primitive: &gltf::Primitive) -> Result<u32, Error> {
    let mut count = None;

    for (_semantic, accessor) in primitive.attributes() {
        if let Some(count) = count {
            if count != accessor.count() {
                bail!("Mesh with different attribute counts");
            }
        }
        else {
            count = Some(accessor.count());
        }
    }

    Ok(count
        .ok_or_else(|| eyre!("Mesh without attributes"))?
        .try_into()
        .unwrap())
}

fn fill_vertex_buffer(
    primitive: &gltf::Primitive,
    blob: &[u8],
    vertex_buffer_view: &mut [Vertex],
    span: &MeshBufferSpan,
) -> Result<(), Error> {
    let positions = primitive
        .get(&gltf::Semantic::Positions)
        .ok_or_else(|| eyre!("Vertex buffer without positions"))?;

    let normals = primitive
        .get(&gltf::Semantic::Normals)
        .ok_or_else(|| eyre!("Vertex buffer without normals"))?;

    //let colors = primitive.get(&gltf::Semantic::Colors(0));

    let uvs = primitive.get(&gltf::Semantic::TexCoords(0));

    let num_vertices = positions.count();

    // note: we already checked that they're equal in `get_num_vertices`, so we can
    // only assert here
    assert_eq!(num_vertices, normals.count());
    if let Some(uvs) = &uvs {
        assert_eq!(num_vertices, uvs.count());
    }
    //if let Some(colors) = &colors {
    //    assert_eq!(num_vertices, colors.count());
    //}

    let mut positions = BufferReader::<[f32; 3]>::new(blob, &positions)?;
    let mut normals = BufferReader::<[f32; 3]>::new(blob, &normals)?;
    let mut uvs = uvs
        .map(|uvs| BufferReader::<[f32; 2]>::new(blob, &uvs))
        .transpose()?;
    //let mut colors = colors
    //    .map(|colors| BufferReader::<[f32; 3]>::new(blob, &colors))
    //    .transpose()?;

    let destination = &mut vertex_buffer_view
        [usize::try_from(span.vertex_buffer_offset).unwrap()..]
        [..usize::try_from(span.num_vertices).unwrap()];

    for vertex in destination.iter_mut() {
        let mut position = Point3::from(positions.next());
        position.z *= -1.0;
        let mut normal = Vector3::from(normals.next());
        normal.z *= -1.0;

        vertex.position = Point3::from(position).to_homogeneous();
        vertex.normal = Vector3::from(normal).to_homogeneous();
        if let Some(uvs) = &mut uvs {
            vertex.uv = Point2::from(uvs.next());
        }
        vertex.texture_id = u32::MAX;
    }

    Ok(())
}

fn fill_index_buffer(
    primitive: &gltf::Primitive,
    blob: &[u8],
    index_buffer_view: &mut [u32],
    span: &MeshBufferSpan,
) -> Result<(), Error> {
    fn copy_index_buffer_inner<S, D>(mut source: BufferReader<S>, destination: &mut [D])
    where
        S: Copy + AnyBitPattern,
        D: From<S>,
    {
        assert_eq!(source.len(), destination.len());

        destination
            .iter_mut()
            .for_each(|x| *x = source.next().into());
    }

    let indices = primitive
        .indices()
        .unwrap_or_else(|| todo!("Mesh without index buffer"));
    let view = indices
        .view()
        .ok_or_else(|| eyre!("Missing view for index buffer"))?;

    let destination = &mut index_buffer_view[usize::try_from(span.index_buffer_offset).unwrap()..]
        [..usize::try_from(span.num_indices).unwrap()];

    match indices.data_type() {
        gltf::accessor::DataType::U16 => {
            copy_index_buffer_inner(BufferReader::<u16>::new_unchecked(blob, &view), destination)
        }
        gltf::accessor::DataType::U32 => {
            copy_index_buffer_inner(BufferReader::<u32>::new_unchecked(blob, &view), destination)
        }
        _ => {
            bail!(
                "Invalid data type for index buffer: {:?}",
                indices.data_type()
            )
        }
    }

    Ok(())
}

fn convert_transform(transform: gltf::scene::Transform) -> LocalTransform {
    match transform {
        gltf::scene::Transform::Matrix { matrix: _ } => {
            // per glTF spec:
            //
            // > When matrix is defined, it MUST be decomposable to TRS properties.
            //
            // so we can implement this
            todo!("Node with matrix transform",);
        }
        gltf::scene::Transform::Decomposed {
            translation,
            rotation,
            scale,
        } => {
                // from the glTF spec:
                //
                // > rotation is a unit quaternion value, XYZW, in the local coordinate system, where W is the scalar.
                //
                // from nalgebra:
                //
                // > This quaternion as a 4D vector of coordinates in the [ x, y, z, w ] storage order.
                //
                // glTF uses a right-handed coordinate system with Z pointing in a different direction than we do, so we need to convert here

                // due to change of handedness we negate X, Y, and Z. and because Z is inverted we negate it again.
                let mut rotation = Quaternion::from(rotation);
                rotation.coords.x *= -1.0;
                rotation.coords.y *= -1.0;
                let rotation = UnitQuaternion::new_unchecked(rotation);

                let mut translation = Translation3::from(translation);
                translation.z *= -1.0;

                if scale != [1.0, 1.0, 1.0] {
                    todo!("scaling");
                }

                LocalTransform {
                    isometry: Isometry3::from_parts(
                        translation,
                        rotation,
                    ),
                }
            }
    }
}

fn import_animation_channel<T, U>(
    blob: &[u8],
    input: &gltf::Accessor,
    output: &gltf::Accessor,
    num_frames: usize,
    mut convert_key_frame: impl FnMut(T) -> U,
    make_channel: impl FnOnce(Vec<KeyFrame<U>>) -> Channel,
) -> Result<Channel, Error>
where
    T: GltfType + AnyBitPattern,
{
    let mut input_buffer = BufferReader::<f32>::new(blob, &input)?;
    let mut output_buffer = BufferReader::<T>::new(blob, &output)?;

    let mut key_frames = Vec::with_capacity(num_frames);

    for i in 0..num_frames {
        let time = input_buffer.next();
        key_frames.push(KeyFrame {
            time,
            value: convert_key_frame(output_buffer.next()),
        });
    }

    Ok(make_channel(key_frames))
}

struct BufferReader<'a, T> {
    blob_slice: &'a [u8],
    stride: usize,
    _marker: PhantomData<fn() -> T>,
}

impl<'a, T> BufferReader<'a, T> {
    fn new_unchecked(blob: &'a [u8], view: &gltf::buffer::View) -> Self {
        let blob_slice = &blob[view.offset()..][..view.length()];
        let stride = view.stride().unwrap_or(size_of::<T>());

        Self {
            blob_slice,
            stride,
            _marker: PhantomData,
        }
    }

    #[inline]
    fn len(&self) -> usize {
        self.blob_slice.len() / self.stride
    }
}

impl<'a, T> BufferReader<'a, T>
where
    T: GltfType,
{
    fn new(blob: &'a [u8], accessor: &gltf::Accessor) -> Result<Self, Error> {
        let view = accessor
            .view()
            .ok_or_else(|| eyre!("Missing view for accessor #{}", accessor.index()))?;
        T::validate(accessor)?;
        Ok(Self::new_unchecked(blob, &view))
    }
}

impl<'a, T> BufferReader<'a, T>
where
    T: AnyBitPattern,
{
    #[inline]
    fn next(&mut self) -> T {
        let value = *bytemuck::from_bytes::<T>(&self.blob_slice[..size_of::<T>()]);
        self.blob_slice = &self.blob_slice[self.stride..];
        value
    }
}

trait GltfType {
    const DATA_TYPE: gltf::accessor::DataType;
    const DIMENSIONS: gltf::accessor::Dimensions;

    #[inline]
    fn validate(accessor: &gltf::Accessor) -> Result<(), Error> {
        if accessor.data_type() != Self::DATA_TYPE {
            bail!(
                "Invalid data type for accessor #{}. Expected {}",
                accessor.index(),
                type_name::<Self>()
            )
        }

        if accessor.dimensions() != Self::DIMENSIONS {
            bail!(
                "Invalid dimensions for accessor #{}. Expected {}",
                accessor.index(),
                type_name::<Self>()
            )
        }

        Ok(())
    }
}

macro_rules! impl_gltf_type_for_primitives {
    ($($ty:ty => $variant:ident,)*) => {
        $(
            impl GltfType for $ty {
                const DATA_TYPE: gltf::accessor::DataType = gltf::accessor::DataType::$variant;
                const DIMENSIONS: gltf::accessor::Dimensions = gltf::accessor::Dimensions::Scalar;
            }

            impl GltfType for [$ty; 2] {
                const DATA_TYPE: gltf::accessor::DataType = gltf::accessor::DataType::$variant;
                const DIMENSIONS: gltf::accessor::Dimensions = gltf::accessor::Dimensions::Vec2;
            }

            impl GltfType for [$ty; 3] {
                const DATA_TYPE: gltf::accessor::DataType = gltf::accessor::DataType::$variant;
                const DIMENSIONS: gltf::accessor::Dimensions = gltf::accessor::Dimensions::Vec3;
            }

            impl GltfType for [$ty; 4] {
                const DATA_TYPE: gltf::accessor::DataType = gltf::accessor::DataType::$variant;
                const DIMENSIONS: gltf::accessor::Dimensions = gltf::accessor::Dimensions::Vec4;
            }
        )*
    };
}

impl_gltf_type_for_primitives!(
    i8 => I8,
    u8 => U8,
    i16 => I16,
    u16 => U16,
    u32 => U32,
    f32 => F32,
);
