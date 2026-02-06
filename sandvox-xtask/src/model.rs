use std::path::Path;

use color_eyre::eyre::{
    Error,
    eyre,
};
use gltf::{
    Gltf,
    Node,
    buffer::Source,
    scene::Transform,
};

pub fn print(path: impl AsRef<Path>, json_output: Option<&Path>) -> Result<(), Error> {
    let gltf = Gltf::open(path)?;

    if let Some(path) = json_output {
        tracing::debug!(?path, "writing JSON to file");
        let json = gltf.as_json().to_string_pretty()?;
        std::fs::write(path, &json)?;
    }

    if let Some(blob) = &gltf.blob {
        println!("- Blob: {} bytes", blob.len());
    }

    let scene = gltf
        .default_scene()
        .ok_or_else(|| eyre!("No default scene"))?;

    fn print_node(node: &Node, depth: usize) {
        print_ident(depth);
        println!("- Node #{}: {:?}", node.index(), node.name());

        let transform = node.transform();
        print_ident(depth);
        println!("  - Transform: {:?}", transform);
        assert!(matches!(&transform, Transform::Decomposed { .. }));

        if let Some(mesh) = node.mesh() {
            print_ident(depth);
            println!("  - Mesh: {:?}", mesh.name());

            for primitive in mesh.primitives() {
                print_ident(depth);
                println!("    - Primitive: {:?}", primitive.mode());

                let aabb = primitive.bounding_box();
                print_ident(depth);
                println!("    - AABB: {aabb:?}");

                if let Some(indices) = primitive.indices() {
                    let view = indices.view().unwrap();
                    let buffer = view.buffer();
                    assert!(
                        matches!(buffer.source(), Source::Bin),
                        "buffer source is an URI"
                    );
                    assert!(view.stride().is_none(), "view has stride");

                    print_ident(depth);
                    println!(
                        "    - Indices: {:?}x{}, {:?}, {}..+{}",
                        indices.data_type(),
                        indices.count(),
                        indices.dimensions(),
                        view.offset(),
                        view.length()
                    );
                }

                for (semantic, attribute) in primitive.attributes() {
                    let view = attribute.view().unwrap();
                    let buffer = view.buffer();
                    assert!(
                        matches!(buffer.source(), Source::Bin),
                        "buffer source is an URI"
                    );
                    assert!(view.stride().is_none(), "view has stride");

                    print_ident(depth);
                    println!(
                        "    - {semantic:?}: {:?}x{}, {:?}, {}..+{}",
                        attribute.data_type(),
                        attribute.count(),
                        attribute.dimensions(),
                        view.offset(),
                        view.length()
                    );
                }
            }
        }

        let children = node.children();
        if children.len() > 0 {
            print_ident(depth);
            println!("  - Children: #{}", children.len());
            for child in children {
                print_node(&child, depth + 2);
            }
        }
    }

    for node in scene.nodes() {
        print_node(&node, 0);
    }

    for animation in gltf.animations() {
        println!("- Animation #{}: {:?}", animation.index(), animation.name(),);

        for channel in animation.channels() {
            let target = channel.target();
            let sampler = channel.sampler();
            println!(
                "  - Channel #{}: {:?}, Node #{:?}, {:?}",
                channel.index(),
                sampler.interpolation(),
                target.node().index(),
                target.property(),
            );
        }
    }

    Ok(())
}

fn print_ident(n: usize) {
    for _ in 0..n {
        print!("  ");
    }
}
