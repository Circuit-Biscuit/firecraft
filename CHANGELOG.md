# Changelog

All notable changes to FireCraft. Each release on GitHub shows the section for its version.

## [0.5.1] - 2026-11-05

The smooth, flowing fire from 0.4 is back. Simulation bounds are now easy to set.

### Added

- **Editable simulation bounds**:
  - Select the Simulation node and its box appears in the viewport. Drag the coloured dot on any face to move that side.
  - *Bounds size* in metres is on the Domain tab.
  - *Edit simulation bounds* is in the Simulation menu and the viewport's right-click menu.

### Changed

- The 0.4 look is the default again. New defaults:
  - Plain ACES tonemapping, with no bloom, no raymarch sharpening and no flame shaping.
  - No vorticity or temperature diffusion.
  - A single-octave noise.
  - A 25% hourglass filter on the reused pressure solve.
- Changing the grid size restarts the simulation only when you let go of the mouse.
- Preset pictures, README screenshots and the preview GIF were re-rendered.

### Fixed

- **Fine grids no longer blow out.** Glow, smoke density and shadows are now measured per metre instead of per voxel. Small voxels used to make the fire brighter and more opaque until it became a flat yellow blob. The look is calibrated to the 0.4 High grid.
- **0.4 projects and presets behave like 0.4 again.** *Buoyancy*, *Burn rate*, *Smoke weight* and the noise *Scale* kept their names in 0.5 but changed units, and old values were read in the new units. For example, buoyancy 1.0 became 1% instead of 300%, so the fire hardly rose and the fuel didn't burn. Old values are now converted. Measured on the GPU, the 0.4 scene's heat, fuel, height and spread now track 0.4.
- Removed the fine dotted grain in thin flames and smoke.

## [0.5.0] - 2026-11-05

Every node now has the full set of settings, organised in tabs like a professional tool.

### Added

- **Simulation**: 14 tabs.
  - **Domain**: voxel count and size, zero point, bounds position and an upscaling option.
  - **Simulation**: combustion, smoke-only or fire-only modes.
  - **Time Control**: timestep, substeps, freeze and looping.
  - **Projection**: quality presets or a custom iteration count.
  - **Combustion**: generated heat, smoke and flames; burn rate; ignition temperature; expansion.
  - **Dissipation**: multiplicative and subtractive dissipation for each channel.
  - **Diffusion** and **Vorticity**: both with temperature, smoke and velocity masks and ranges.
  - **Force**: buoyancy, smoke weight, fuel weight, gravity and solid walls on each side.
  - **Wind**, **Shredding**, **Mask** and **Visual** (a domain gizmo).
- **Emitter: Volume**: rotation, random pressure (intensity, scale, seed, speed), velocity transfer when the emitter moves, an emitter colour, and the option to emit light into the scene.
- **Force nodes**:
  - Shared settings: activity, mode (volume, particles or both), rotation, masks with ranges and softness, tethered mask shapes, a mask outline, and arrows that show the force in the viewport.
  - **Force: Noise** adds scale in cm, seed, octaves, lacunarity, gain, amplitude, bias, animation speed, a curl option and a live noise preview.
- **Shading**: tabs for Lighting, Scattering, Emissive masking, Flames, Smoke, Fuel and Rendering, with live **temperature and smoke histograms**. It adds flame shaping, remap ranges and ramps, extinction colour, shadow controls and sharpening.
- **Fire Color Gradient**: a full gradient editor. Click to add a stop, drag to move it, right-click to remove it. Interpolates in RGB, Linear, HSV or OKLab, and imports or exports `.fcgrad` files or a PNG strip.
- **Scene**:
  - **Tonemapping**: None, Reinhard, ACES, Filmic, Uchimura or AgX; gamma, exposure, vibrance, contrast, brightness, saturation and dithering.
  - **Colors**: channel swizzle and bloom.
  - Vignetting.
  - Ten **render styles**: Realistic, Pixelized, Edge, Chromatic Aberration, Kuwahara, Anisotropic Kuwahara and four stroke styles; also quantisation and remapping.
- **Volume Processing**: motion blur, sharpen or dilate as a post process; post modulation; a slicing mask.
- **Render node**:
  - **Render passes**: Render All, Alpha, Emission, Scattering, Temperature, Density, Fuel or Depth, each written to RGBA, RGB or one channel, with optional HDR.
  - **Export**: flipbook or image sequence with sizes and a visual grid editor; first frame, frame count, stride, numbering offset and absolute frame numbers; filename patterns such as `$(project)_$(safename)`; **PNG, EXR and TGA**; *Export Now* and *Open Folder*.
  - **Render Settings**: alpha blending mode, inverted alpha, shape rendering and masking, and occlusion between particles and volume.
- The viewport overlay shows **Projection Quality**. The orthographic camera moved to the *View* menu.

### Changed

- New bloom and tonemapping give a cleaner fire. The core no longer burns out to white, and there is no glow halo.
- Flames now fade as the gas cools, so the default fire is compact and no longer trails a red cloud.
- All 44 preset pictures were re-rendered with the new engine.
- Projects and presets from 0.4 still open. Old setting names are translated to the new ones automatically.

### Fixed

- Smoke near warm gas was lit pink. Flame glow now comes only from real flames.

### Known limitations

- *Import control* checkboxes and *Force Tightness* have no effect yet.
- Several advanced settings are approximations of their described behaviour.

## [0.4.0] - 2026-11-05

First public build.

- Real-time 3D fire, smoke and fuel simulation on the GPU, with combustion, colliders and wind.
- A node graph with the full node library: shapes, emitters, colliders, forces, modulators, lights, cameras, colours, import and export.
- A GPU ray-marched viewport with shadows, lights, sparks and HQ refinement.
- 44 distinct presets with pre-rendered pictures, a home screen and recent projects.
- Undo and redo, clipboard, and project files.
- Image sequence, flipbook and volume export.
- A Windows installer, a macOS universal app, a Linux AppImage, and automatic releases.
