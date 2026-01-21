# Voxel experiments

This is yet another toy voxel engine. It's written in Rust with [wgpu][1].

At the time of writing voxels are stored in chunks, which are backed by plain arrays in Morton order. For rendering they're meshed using [Greedy meshing][2]. So far I kept it somewhat modular, because I'd like to experiment with different voxel data structures and rendering approaches, and benchmark them against each other.

![Screenshot of the sandvox, showing a simple voxel landscape](https://media.githubusercontent.com/media/jgraef/sandvox/refs/heads/main/doc/screenshot.png)

[1]: https://docs.rs/wgpu/latest/wgpu/
[2]: https://0fps.net/2012/06/30/meshing-in-a-minecraft-game/
