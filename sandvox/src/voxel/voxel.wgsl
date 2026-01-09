
@group(0)
@binding(0)
var<storage, read_write> nodes: array<u64>;

@group(0)
@binding(1)
var<storage, read_write> leaves: array<u64>;

struct BuildCommand {
    levels: u32,
    length: u32,
}

@group(1)
@binding(0)
var<uniform> input_command: BuildCommand;

@group(1)
@binding(1)
var<storage, read> input_data: array<u64>;


@compute
@workgroup_size(64)
fn voxel_build() {

}
