# Changelog

All notable changes to FireCraft. Each release on GitHub shows the section for its version.

## [0.6.0] - 2026-11-05

The 0.4.0 look is back, with the node settings from 0.5 rebuilt on top of it, and the blocky glitches are gone.

### Changed

- **Back to the 0.4.0 fire engine.** The 0.5.0 simulation and renderer made the fire grainy and blotchy, so they are replaced by 0.4.0's. The Simulation node has 0.4.0's settings again.
- The look is guarded by a test. It renders the default fire and a smoky outdoor scene, and compares them pixel by pixel with pictures made by the 0.4.0 build.
- **New defaults for every preset:**
  - High quality, 10 pressure iterations and 2 substeps.
  - 100% shadowing sharpness.
  - Dilate post processing and raymarch sharpening.

### Added

Every new setting leaves the picture unchanged until you move it off its default.

- **Fire Color Gradient**: a full gradient editor.
  - Click the bar to add a stop, drag a stop to move it, right-click a stop to remove it.
  - Interpolates in RGB, Linear RGB, HSV or OKLab.
  - Imports and exports `.fcgrad` files, or exports a PNG strip.
- **Shading**: tabs for Lighting, Scattering, Emissive masking, Flames, Smoke, Fuel and Rendering.
  - Live temperature and smoke histograms.
  - Coloring remap ranges and ramps, flame shaping, and flame colouring by gradient, blackbody or a single colour.
  - Translucency, density limits, smoke colouring by gradient, and fuel rendering.
  - Shadow sharpness and intensity, extinction colour, lighting anisotropy and ambient occlusion.
  - Light from flames and particles, emissive masking, and raymarch sharpening.
- **Scene**:
  - Tonemapping: None, Reinhard, ACES, Filmic, Uchimura or AgX; gamma, exposure, vibrance, contrast, brightness, saturation and dithering.
  - Colors: channel swizzle and bloom.
  - Vignetting.
  - Ten rendering styles, with quantization and remapping styles.
- **Volume Processing**:
  - Post process: Motion Blur, Sharpen or Dilate.
  - Post modulation.
  - A slicing mask that fades the volume towards each wall.
- **Render node**:
  - Render passes: Render All, Alpha, Emission, Scattering, Temperature, Density, Fuel or Depth. Each is written as RGBA, RGB or a single channel, and can be HDR.
  - Export as a flipbook or an image sequence.
  - Flipbook layout on a visual grid.
  - First frame, frame count, stride, numbering offset and absolute frame numbers.
  - Filename patterns such as `$(project)_$(safename)`.
  - PNG, EXR and TGA files.
  - *Export Now* and *Open Folder*.
  - Alpha blending, inverted alpha, and shape holdout or masking.
  - Whether the volume and particles hide each other.
- **Force nodes (Fire Chaos, Emission Chaos)**:
  - Scale in cm, seed, octaves, lacunarity, gain, bias, animation speed and curl noise.
  - Rotation.
  - Temperature, smoke and velocity masks with ranges.
  - Mode: Volume, Particles or both.
  - Arrows in the viewport that show the force field, and the mask outline.
- **Emitter: Volume**:
  - Rotation.
  - Random pressure.
  - Velocity transfer from a moving emitter.
  - Show emitter, Shadow caster, and Emit light.
- **Fire Shape**: shape activity, rotation and *Fill motion gaps*.

### Fixed

- **Blocky glitches.** The renderer skips empty space in blocks of 8×8×8 voxels. It counted thin haze as empty, which cut rectangular holes into smoke. It could also jump past the start of a filled block and cut that block's edge off. Both are fixed.

### Known limitations

- The *Import control* checkboxes have no effect yet.
- Sharpening, ambient occlusion, motion blur and particle occlusion are simplified versions of the original's behaviour.
- The preset pictures are still 0.4's renders.

## [0.5.0] - 2026-11-05

- Every node had the full set of settings, in tabs.
- New simulation and rendering engine. 0.6.0 replaced it, because it made the fire grainy.

## [0.4.0] - 2026-11-05

First public build.

- Real-time 3D fire, smoke and fuel simulation on the GPU, with combustion, colliders and wind.
- A node graph with the full node library: shapes, emitters, colliders, forces, modulators, lights, cameras, colours, import and export.
- A GPU ray-marched viewport with shadows, lights, sparks and HQ refinement.
- 44 distinct presets with pre-rendered pictures, a home screen and recent projects.
- Undo and redo, clipboard, and project files.
- Image sequence, flipbook and volume export.
- A Windows installer, a macOS universal app, a Linux AppImage, and automatic releases.
