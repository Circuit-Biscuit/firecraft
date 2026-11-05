# Changelog

All notable changes to FireCraft. Each release on GitHub shows the section for its version.

## [0.6.0] - 2026-11-05

The fire engine from 0.4.0 is back, unchanged. Every node and setting from 0.5 stays.

### Changed

- **The fire and smoke look exactly like 0.4.0 again.** The 0.5 simulation and renderer are replaced by the 0.4.0 engine. A test renders the default scene and a 0.4 project, then compares every pixel with pictures made by the 0.4.0 build. They match exactly: 0 of 255 difference on every pixel.
- The new node settings drive the 0.4 engine. Every setting that existed in 0.4 converts back to exactly the number 0.4 used.
- Settings the 0.4 engine has no counterpart for yet are shown dimmed. Hovering over one says so. They will be connected one at a time, each checked against the 0.4 pictures.
- The preset pictures and README media are 0.4's own again.

### Added

- **Editable simulation bounds**:
  - Select the Simulation node and drag the coloured dots on the box in the viewport.
  - Or type the *Bounds size* in metres on the Domain tab.
  - *Edit simulation bounds* is also in the Simulation menu.
- The 0.4 engine now takes any voxel count, voxel size and box position.
- It also gains the Shading node's temperature and smoke histograms.
- Export render passes are added to the 0.4 renderer: alpha, emission, scattering, temperature, density, fuel and depth. You can also choose premultiplied, straight or opaque alpha, invert the alpha, and save PNG, EXR (linear colour) or TGA.

### Fixed

- 0.4 projects and presets: *Buoyancy*, *Burn rate*, *Smoke weight* and the noise *Scale* kept their names in 0.5 but changed units. Their old values are now converted on load.

### Known limitations

- Not connected yet (dimmed):
  - Tonemapping choices, bloom, vignette and styles.
  - Flame shaping and emissive masking.
  - Diffusion, vorticity masks, shredding, the domain mask, solid walls other than the ground, and wind chaos.
  - Advection and projection options other than iterations.
  - Rotation of emitters, shapes and forces, and the force masks by temperature, smoke or velocity.
  - The noise force's octaves, lacunarity, gain, bias and seed.
- The 0.5 engine is kept on the branch `new-engine-0.5.1`.

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
