This is the Linux backend from the crates.io release of
`gpui-linux-gpui-unofficial` 1.29.3, under the included Apache 2.0 license.

Saiki patches `src/linux/wayland/window.rs` so non-resizable Wayland toplevels
publish matching minimum and maximum dimensions before mapping. Tiling
compositors such as Hyprland can then open the controls as a floating window.
Remove the patch when the pinned GPUI release supports these size hints.
