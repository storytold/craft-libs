# Material graph text format

`craft-material-graph` parses text material graphs and evaluates them to float RGBA images. The loose file extension
is `.tcgraph`. This page describes format 3, the version written by `graph::FORMAT_VERSION`.

## Lines

One directive per line. `#` starts a comment that runs to the end of the line; blank lines are ignored. Values are
whitespace-separated tokens; a value with spaces is written in double quotes.

| Directive | Meaning |
|---|---|
| `format <n>` | The format version the text was written with. Texts without it are version 1. A version newer than the runtime's is refused with an error. |
| `size <w> [<h>]` | The graph's nominal size in pixels (the height defaults to the width). An evaluation may ask for another size (1 to 8192); node results scale with it. |
| `seed_offset <n>` | Added to every random seed in the graph. |
| `meta <key> "<value>"` | Free key/value data. `meta kind "function"`, `"fxmap"` or `"patterns"` marks a subgraph as a function graph, an FX-map program or a pattern program; `meta label` names the graph. |
| `node <id> <kind> [key=value ...]` | A node. `id` is a whole number, unique in its graph; `kind` is one of the node kinds below. `key=@<id>` connects input slot `key` to node `id`, `key=@<id>.<output>` to a named output of a node with several. A number value is a parameter; anything else is a text parameter. `_x=` and `_y=` are the node's position in an editor. |
| `output <name> <id>[.<output>]` | A graph output named `name`, taken from node `id`. Graphs may instead carry `output` nodes (`usage`, `identifier`). |
| `subgraph <name>` ... `end` | A graph defined inside this one, in the same format, used by `subgraph` nodes (`graph="<name>"`), by `pixel_processor`/`value_processor` nodes (`fn="<name>"`), by `fx_map` (`program="<name>"`) or by `pattern_program` (`program="<name>"`). |
| `resource <name> <base64>` | Bytes carried by the text (an image a `bitmap` node reads by `file="<name>"`). |
| `expose <label> <node> <param> <min> <max>` | A parameter shown to users under `label`, clamped to the range. |
| `link <name> "<path>" ["<graph>"]` | A live link: the graph is read from another file (relative to this one) whenever the text is loaded, and used through `subgraph` nodes named `name`. Native builds only. |
| `comment <x> <y> <w> <h> <color> <text>` | An editor note; ignored by evaluation. |

Example:

```text
format 3
size 256
node 1 perlin scale=8 seed=3
node 2 levels in=@1 low=0.2 high=0.8
node 3 blend bg=@1 fg=@2 mode="multiply" opacity=0.5
output height 3
```

## Named values

A node keeps these as numbers in memory; the text writes a name where one exists. Reading accepts the name or the
number. The tables are in `src/names.rs`.

| Node kind | Key | Names |
|---|---|---|
| every kind | `format` (output format) | `graph` (the graph's own), `u8`, `u16`, `f16`, `f32`, and `u8_compiled`, `u16_compiled`, `f16_compiled`, which quantise the way compiled graphs do (their own rounding, and blurs and levels that keep the format on the way) |
| `blend` | `mode` | `copy`, `add`, `subtract`, `multiply`, `screen`, `overlay`, `max`, `min`, `difference`, `soft_light`, `color_dodge`, `color_burn`, `linear_burn`, `hard_light`, `exclusion`, `divide`, `linear_light`, `color`, `luminosity`, `dissolve`, `add_sub`, `switch`, and `add_compiled`, `subtract_compiled`, `add_sub_compiled`, `divide_compiled`, `soft_light_compiled` (opacity acts before the result is limited) |
| `pp_quadrant` | `pattern` | `square`, `disc`, `paraboloid`, `bell`, `gaussian`, `thorn`, `pyramid`, `brick`, `gradation`, `waves`, `half_bell`, `ridged_bell`, `crescent`, `capsule`, `cone`, `gradation_offset`, `hemisphere` |
| `f_random` | `lcg` | which 24-bit generator a draw comes from, by where the function runs: `pixel`, `value`, `parameter`, `size`, `fx_luminance`, `fx_branch_offset`, `fx_pattern_offset`, `fx_size`, `fx_rotation`, `fx_variation`, `fx_switch_condition`, `fx_iterate_count`, `fx_image_index` |

Older spellings still read the same: the key `ofmt=<n>` for `format`, numbers in place of the names, and seven function
ops under their format-2 names (`f_fsin`, `f_fcos`, `f_flog`, `f_fdiv`, `f_fatan2`, `f_gsin`, `f_gcos`, now
`f_sin_poly`, `f_cos_poly`, `f_log_floor`, `f_div_large`, `f_atan2_approx`, `f_sin_f32turns`, `f_cos_f32turns`).

## Evaluation

- Results are float RGBA. The same text, size and parameters give the same bits on every platform and thread count:
  transcendental functions come from one implementation (`crate::math`), parallel loops split work at fixed indices,
  and results never depend on which thread computed them.
- Evaluation never panics on a hostile text: sizes are limited to 8192 per side, subgraphs nest at most 8 deep, function
  calls at most 8 deep, a function graph runs at most 2048 nodes deep per pixel, FX-maps draw at most 500,000 markers,
  and loops are bounded.
- The host stops an evaluation with a cancel flag (`context::Cancel`); the runtime never reads a clock. Files a graph
  names but does not carry are read through the host's `context::FileSource` (none on the web).
- `versions::of_kind` gives a version per node kind, derived at build time from the source each kind reaches, for
  caches of results.

## Node kinds

Function graphs (`meta kind "function"`) carry values (float, float2-4, integer, integer2-4, boolean, string) instead of
images and never produce NaN or infinity. FX-map programs (`meta kind "fxmap"`) are run by the `fx_map` node; pattern
programs (`meta kind "patterns"`) by `pattern_program`. The tables below are the runtime's own node definitions
(`graph::SPECS`, `compiled_ops::SPECS`, `atomic::SPECS`, `func::FUNC_SPECS`, `fxmap::FXM_SPECS`, `patterns::SPECS`).

### Image nodes

| Node | Category | Inputs | Description and parameters |
|---|---|---|---|
| `color` | Generators |  | constant colour: r g b a |
| `white_noise` | Generators |  | per-pixel random: seed distribution(0 uniform,1 gaussian,2 triangle) |
| `gaussian_noise` | Generators |  | gaussian-distributed per-pixel noise: seed scale contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `perlin` | Generators |  | perlin noise: scale scale_x scale_y seed disorder disorder_speed tile_x tile_y |
| `fbm` | Generators |  | fractal noise/clouds: scale octaves gain seed mode(0 fbm,1 ridged,2 turbulence) disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `voronoi` | Generators |  | cellular: scale jitter seed mode(0 f1,1 cell id,2 edge,3 f2,4 f2-f1,5 f1*f2,6 f1/f2) aspect offset_x offset_y distortion distortion_scale distance_mode(0 euclid,1 manhattan,2 chebyshev,3 minkowski) minkowski edge_thickness rounded distance_scale color_seed_mode color_seed invert |
| `voronoi_fractal` | Generators |  | layered voronoi: scale jitter seed mode aspect offset_x offset_y distortion distortion_scale distance_mode minkowski edge_thickness rounded distance_scale min_level max_level roughness lacunarity opacity blend(0 add,1 max,2 min) color_seed_mode color_seed invert |
| `gradient` | Generators |  | gradient: mode(0 linear,1 radial) angle |
| `circle` | Generators |  | radius softness |
| `rect` | Generators |  | width height softness |
| `polygon` | Generators |  | sides radius softness rotation |
| `checker` | Generators |  | tiles |
| `bricks` | Generators |  | rows cols mortar bevel offset lum_var seed |
| `fx_map` | Generators | in (optional), in2 (optional), in3 (optional), in4 (optional) | run a pattern program: program="name" depth seed blend (0 max 1 add 2 over 3 min 4 multiply) color bg bg_r bg_g bg_b |
| `curve` | Filters | in | tone curves: points (master), points_r points_g points_b points_a as "x:y,x:y" lists |
| `svg` | Generators |  | vector image: file="art.svg" fit (0 contain 1 stretch 2 cover) scale x y |
| `text` | Generators |  | text: text="..." font="face.ttf" (optional) size x y align (0 left 1 centre 2 right) spacing line_spacing rotation value |
| `channel_shuffle` | Filters | in, in2 (optional) | rebuild channels: src_r src_g src_b src_a (0-3 channels of in, 4-7 channels of in2, 8 zero, 9 one), inv_r inv_g inv_b inv_a |
| `input_value` | Utility |  | graph parameter read by functions: name, type (0 float, 1 int, 2 bool, 3 float2, 4 float3, 5 float4, 6 int2, 7 int3, 8 int4, 9 string), v / x y z w, min max, label |
| `pixel_processor` | Generators | in (optional), in2 (optional), in3 (optional), in4 (optional), in5 (optional), in6 (optional), in7 (optional), in8 (optional), in9 (optional), in10 (optional), in11 (optional), in12 (optional), in13 (optional), in14 (optional), in15 (optional), in16 (optional), in17 (optional), in18 (optional), in19 (optional), in20 (optional), in21 (optional), in22 (optional), in23 (optional), in24 (optional), in25 (optional), in26 (optional), in27 (optional), in28 (optional), in29 (optional), in30 (optional), in31 (optional), in32 (optional) | per-pixel expr / expr_g / expr_b / expr_a (string params), or fn="function" to run a function graph per pixel |
| `value_processor` | Generators | in (optional), in2 (optional), in3 (optional), in4 (optional) | run fn="function" once; the result (float, float3 or float4) fills the image. The function can read image statistics |
| `sdf_shape` | SDF |  | 2D signed distance shape: shape(0 circle,1 box,2 polygon,3 ring) size aspect sides rotation round x y range |
| `sdf_op` | SDF | a, b | combine SDFs: op(0 union,1 subtract,2 intersect) smooth range |
| `sdf_mask` | SDF | in | SDF -> mask: offset softness |
| `sdf_form` | SDF |  | procedural 3D SDF form as height: shape(0 sphere,1 box,2 torus,3 cylinder) size repeat round scatter seed height |
| `bitmap` | Utility |  | flip_x flip_y gray; imported PNG bitmap: file="path.png" |
| `input` | Utility |  | subgraph input: idx |
| `subgraph` | Utility | in0 (optional), in1 (optional), in2 (optional), in3 (optional) | instance of a reusable subgraph: graph="name" |
| `shape` | Generators |  | procedural shape: square, disc, paraboloid, gaussian, diamond, pyramid, half sphere, cone |
| `tile_sampler` | Generators | pattern (optional), pattern2 (optional), pattern3 (optional), pattern4 (optional), pattern5 (optional), pattern6 (optional), mask (optional), scale_map (optional), rotation_map (optional), displacement_map (optional), color_map (optional), distribution_map (optional), background (optional) | grid of pattern instances: x_amount y_amount size aspect size_mode scale scale_random size_random rotation rotation_random pattern_rotation symmetry_random position_random offset offset_mode offset_random global_offset_x global_offset_y color color_random lum_by_number lum_by_scale mask_threshold mask_invert mask_random mask_checker invert_mask blending opacity background reverse_order pattern_type pattern_specific pattern_distribution |
| `tile_generator` | Generators | pattern (optional), pattern2 (optional), pattern3 (optional), pattern4 (optional), pattern5 (optional), pattern6 (optional), mask (optional), scale_map (optional), rotation_map (optional), displacement_map (optional), color_map (optional), distribution_map (optional), background (optional) | brick-style tiles: x_amount y_amount size aspect size_mode scale scale_random size_random rotation rotation_random pattern_rotation symmetry_random position_random offset offset_mode offset_random global_offset_x global_offset_y color color_random lum_by_number lum_by_scale mask_checker mask_horizontal mask_vertical mask_random invert_mask blending opacity background reverse_order pattern_type pattern_specific pattern_distribution |
| `splatter` | Generators | pattern (optional), pattern2 (optional), pattern3 (optional), pattern4 (optional), pattern5 (optional), pattern6 (optional) | random scatter of a pattern: count grid_number scale scale_var aspect rotation rot_var pattern_rotation lum_var disorder pan_x pan_y symmetry_random gain output_min output_max background blending opacity pattern_type pattern_specific pattern_distribution |
| `splatter_circular` | Generators | pattern (optional), pattern2 (optional), pattern3 (optional), pattern4 (optional), pattern5 (optional), pattern6 (optional) | instances on one or more circles: number amount_random rings radius radius_random ring_radius_multiplier angle_random spiral_factor spread size size_random scale_by_number scale_by_ring rotation rotation_random center_orientation ring_rotation ring_rotation_random position_random color luminance_random lum_by_number lum_by_ring mask_random background blending opacity pattern_type pattern_specific pattern_distribution |
| `clouds` | Generators |  | soft cloud noise: scale seed contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `bnw_spots` | Generators |  | black and white spots: scale seed contrast invert variant disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `crystal` | Generators |  | faceted crystal cells: scale disorder seed contrast invert variant |
| `directional_noise` | Generators |  | streak noise, seamless at any angle: scale stretch angle seed contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `grunge` | Generators |  | grunge masks: variant(0 concrete,1 scratches,2 spots,3 smudges) balance large_variation base_noisiness dirt_specks_opacity invert_dirt scratches_opacity sharpen quantity scratch_width scratch_length details coverage coverage_contrast disorder disorder_speed seed contrast invert |
| `plasma` | Generators |  | smooth plasma waves |
| `stripes` | Generators |  | stripes: count angle softness width tile(1 = snap to seamless angle, 0 = exact) |
| `weave` | Generators |  | woven threads: count width |
| `star` | Generators |  | star shape: points inner size softness rotation |
| `gradient_axial` | Generators |  | symmetric gradient: angle |
| `gradient_circular` | Generators |  | angular gradient: offset |
| `switch` | Filters | a (optional), b (optional), c (optional), d (optional) | pass one of up to four inputs: index |
| `flood_fill` | Filters | in | label connected regions of a mask |
| `flood_fill_random` | Filters | in | random grayscale per flood-fill region |
| `flood_fill_color` | Filters | in | random color per flood-fill region |
| `flood_fill_to_index` | Filters | in | region number of each flood-fill region as a gray level |
| `flood_fill_to_position` | Filters | in | centre of each flood-fill region (outputs: position, x, y) |
| `flood_fill_to_bbox_size` | Filters | in | bounding-box size of each flood-fill region (outputs: size, width, height) |
| `flood_fill_gradient` | Filters | in | gradient across each flood-fill region |
| `vector_warp` | Filters | in, vector | displace by a vector map: intensity directx |
| `slope_blur` | Filters | in, slope | blur along a slope map: samples intensity mode |
| `blur_hq` | Filters | in | high quality blur: intensity quality |
| `histogram_range` | Filters | in | select a range of values: position range contrast invert |
| `bevel` | Filters | in | bevel a shape: distance smoothing threshold use_as_normal normal_intensity corner(0 round,1 angular) directx |
| `distance` | Filters | in | distance field of a mask: distance mode |
| `color_match` | Filters | in, reference (optional), src_color (optional), target_color (optional) | recolour: method(0 statistics transfer from reference,1 colour swap) strength source_mode(0 average,1 colour,2 input) src_r src_g src_b target_mode(0 colour,1 input) target_r target_g target_b custom_variation variation_hue variation_chroma variation_luma use_mask mask_hue_range mask_chroma_range mask_luma_range mask_smoothness |
| `curvature_smooth` | Material | in | smooth curvature from a normal map |
| `height_blend` | Material | bg, fg, mask (optional) | blend two height maps: opacity contrast output |
| `make_it_tile_photo` | Filters | in | make a photo tile: width width_v |
| `make_it_tile_patch` | Filters | in | make an image tile with random patches |
| `clamp` | Filters | in | clamp values: low high apply_alpha |
| `highpass` | Filters | in | high-pass filter: radius |
| `swirl` | Filters | in | swirl distortion: amount radius |
| `edge_wear` | Material | in, noise (optional) | edge wear mask from a height map: amount width contrast |
| `dirt` | Material | in, noise (optional) | dirt mask from an ambient occlusion map: amount contrast |
| `dust` | Material | in, noise (optional) | dust mask from a normal map: amount contrast |
| `output` | Utility | in (optional) | graph output: usage identifier |
| `dot` | Utility | in | re-route a link |
| `blur` | Filters | in | gaussian blur: radius |
| `directional_blur` | Filters | in | angle length |
| `sharpen` | Filters | in | radius amount |
| `levels` | Filters | in | low high gamma out_low out_high |
| `adjust` | Filters | in | brightness contrast |
| `invert` | Filters | in | invert colours: invert |
| `threshold` | Filters | in | binary threshold: threshold softness mode(0 greater,1 greater or equal,2 lower,3 lower or equal) |
| `histogram_scan` | Filters | in | position width |
| `quantize` | Filters | in | steps |
| `to_gray` | Filters | in | luminance |
| `channel` | Filters | in | ch(0 r,1 g,2 b,3 a) |
| `merge` | Filters | r (optional), g (optional), b (optional), a (optional) | combine channels |
| `tint` | Filters | in | multiply by colour: r g b |
| `colorize` | Filters | in | gradient map: interp(0 linear,1 smooth,2 step,3 smoother) smoothness addressing(0 clamp,1 repeat) use_alpha  stops p0 r0 g0 b0 a0 ... p15 r15 g15 b15 a15 |
| `hsl` | Filters | in | hue sat val |
| `transform` | Filters | in | offset_x offset_y scale_x scale_y rotation(turns) |
| `mirror` | Filters | in | mirror an image: axis(0 x,1 y,2 both,3 corner) offset_x offset_y invert_x invert_y corner(0 top left,1 top right,2 bottom left,3 bottom right) |
| `warp` | Filters | in, warp | slope warp: intensity filtering(0 bilinear,1 nearest,2 bicubic) |
| `directional_warp` | Filters | in, warp | warp in one direction: angle intensity offset filtering(0 bilinear,1 nearest,2 bicubic) |
| `blend` | Filters | fg, bg, mask (optional) | blend two images: mode (copy, add, subtract, multiply, screen, overlay, max, min, difference, soft_light, color_dodge, color_burn, linear_burn, hard_light, exclusion, divide, linear_light, color, luminosity, dissolve, add_sub, switch, and as compiled graphs blend: add_compiled, subtract_compiled, add_sub_compiled, divide_compiled, soft_light_compiled) opacity alpha_mode(0 source alpha,1 ignore alpha,2 straight,3 premultiplied) crop_x0 crop_y0 crop_x1 crop_y1 |
| `normal` | Material | in | height->normal (OpenGL +Y): strength directx alpha_from_input |
| `normal_blend` | Material | a, b, mask (optional) | blend normal maps (a = background, b = foreground): opacity |
| `ao` | Material | in | ambient occlusion: mode(0 blur,1 horizon) radius intensity hbao_radius hbao_intensity quality(0 4,1 8,2 16 samples) use_world_units height_depth surface_size height_depth_cm |
| `quantize_color` | Filters | in | reduce to few colours (k-means): color_amount contour_smoothing dithering dithering_pattern(0 noise,1 bayer) ignore_alpha distance_color_space(0 lab,1 rgb) |
| `curvature` | Material | in | curvature of a normal map: mode(0 normal map,1 height map) intensity directx radius |
| `edge_detect` | Filters | in | edge lines: mode(0 lines,1 sobel) edge_width edge_roundness invert tolerance intensity |
| `emboss` | Filters | in | angle intensity |
| `non_uniform_blur` | Filters | in, mask | blur by a mask: intensity |
| `radial_blur` | Filters | in | spin blur around a centre: mode(0 spin,1 zoom) samples angle x y amount radius |
| `anisotropic_blur` | Filters | in | blur stretched in one direction: intensity anisotropy angle |
| `symmetry` | Filters | in | mirror the image around a line: axis(0 x,1 y,2 both,3 diagonal,4 anti-diagonal,5 eight-fold) offset flip transfer(0 copy,1 add,2 sub,3 mul,4 add sub,5 max,6 min) |
| `skew` | Filters | in | shear that keeps tiling: skew_x skew_y (whole numbers) align(0 centre,1 top left,2 bottom right) |
| `auto_levels` | Filters | in | stretch values to the full range: per_channel |
| `channel_mixer` | Filters | in | mix colour channels: rr rg rb gr gg gb br bg bb mono |
| `alpha_merge` | Filters | rgb, alpha (optional) | put a grayscale image into the alpha channel |
| `color_to_mask` | Filters | in | mask of pixels close to a colour: r g b range softness |
| `replace_color` | Filters | in | replace one colour with another: src_r src_g src_b dst_r dst_g dst_b range softness |
| `pow` | Filters | in | raise values to a power: exponent |
| `histogram_shift` | Filters | in | shift all values: position |
| `grayscale_conversion` | Filters | in | weighted grayscale: r g b |
| `cells` | Generators | color (optional) | cellular noise: scale jitter seed variant(0 cells1,1 cells2,2 cells3,3 cells4) luminance_random color_source(0 random,1 pseudorandom,2 image) pseudorandom_seed edge_width hardness contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `fractal_sum` | Generators |  | sum of noise octaves: scale min_level max_level roughness seed mode(0 base,1 turbulence,2 creased) contrast invert disorder disorder_speed |
| `waveform` | Generators |  | tileable wave pattern: count angle warp warp_scale seed shape(0 sine,1 triangle,2 saw,3 square) contrast invert disorder disorder_speed |
| `gaussian_spots` | Generators |  | soft gaussian spots: scale size size_variation seed contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `normal_invert` | Material | in | flip normal map channels: invert_x invert_y invert_z invert_a |
| `normal_normalize` | Material | in | renormalise a normal map |
| `normal_vector_rotation` | Material | in, rotation (optional) | rotate normal vectors: angle directx |
| `normal_to_height` | Material | in | integrate a normal map into a height map: directx relief_balance opacity |
| `sobel` | Filters | in | sobel edge magnitude: intensity |
| `glow` | Filters | in | glow around bright areas: width intensity clear glow_r glow_g glow_b |
| `mosaic` | Filters | in, mosaic_map (optional) | mosaic of the map regions (or a pixel grid without a map): count intensity |
| `non_uniform_directional_warp` | Filters | in, intensity, angle (optional) | warp along a direction chosen per pixel: intensity angle |
| `multi_directional_warp` | Filters | in, warp | warp in several directions at once: intensity directions(0 one,1 two,2 four) angle mode(0 average,1 max,2 min,3 chain) |
| `normal_combine` | Material | base, detail | combine two normal maps: technique(0 whiteout,1 channel mixer,2 detail oriented) |
| `triangle_grid` | Generators |  | grid of triangles: style(0 squares,1 jittered mesh) count border variation x_amount y_amount random_position_multiplier quincunx_offset_x quincunx_offset_y triangulation(0 fixed,1 alternate,2 random) output(0 smooth,1 flat,2 edge distance) height_random thin_lines distance_to_edge_thickness thin_line_color thin_line_background seed contrast invert |
| `grunge_map` | Generators |  | grunge map 1-15: map balance contrast brush_pattern cracks disorder coverage seed invert |
| `rgba_split` | Filters | in | split into R, G, B and A grayscale outputs |
| `alpha_split` | Filters | in | split into an RGB output and an alpha output |
| `weave_generator` | Generators |  | woven threads with gaps: tile_x tile_y weave shape gap_x gap_y contrast invert |
| `starburst` | Generators |  | star-like rays: count rays sharpness size rotation rotation_random seed contrast invert |
| `gradient_reflected` | Generators |  | gradient that goes 0-1-0: count angle contrast invert |
| `shape_stroke` | Filters | in | outline of a shape: width offset softness threshold opacity color_r color_g color_b |
| `shape_glow` | Filters | in | glow around a shape: width softness intensity threshold opacity spread color_r color_g color_b |
| `chrominance_extract` | Filters | in | colour without brightness |
| `shape_drop_shadow` | Filters | in | drop shadow of a mask: angle distance blur opacity spread |

### Compiled-graph image nodes

| Node | Inputs | Description and parameters |
|---|---|---|
| `curve_keys` | in | tone curve from Bezier keys: n, then per key k xk yk (point) lxk lyk (left handle) rxk ryk (right handle); ramp=1 draws the curve left to right instead of applying it |
| `triangle_strips` |  | white triangle strips on black: strips="...", each strip hex digits, 8 per vertex (yyyyxxxx, 0..ffff across the image), strips apart by a space |
| `distance_map` | in, source (optional) | distance from the mask's bright pixels: distance (reach in 1/256 of the image, default 10), aspect_x aspect_y, tile_x tile_y (1, the default: across the image's edge); with a source, the colour of the nearest mask pixel, fade=1 fading it out with distance |
| `normal_map` | in | tangent-space normals from a height: intensity (default 1, scaled by width/256), flip_y, alpha=1 keeps the height in alpha, aspect_x aspect_y |
| `emboss_map` | in, map | adds the slope of map along a direction: intensity (default 1, scaled by width/256), angle (turns), aspect_x aspect_y |
| `gradient_dynamic` | in, grad | look up the grad image with the input: in picks the column (vertical=1 the row), position the other coordinate |

### Atomic image nodes

| Node | Inputs | Description and parameters |
|---|---|---|
| `levels_mid` | in | levels with a midpoint: low high mid out_low out_high; per channel with suffix _r _g _b _a (e.g. low_r) |
| `hsl_centered` | in | hue, saturation and lightness shifts, 0.5 = unchanged, twice the distance from 0.5 is added: hue (turns) sat lum |
| `transform_matrix` | in | transform by a matrix around the centre, reading the input at (m00 x + m10 y + offset_x, m01 x + m11 y + offset_y) from the centre: tile_x tile_y (1 repeat, 0 bg_r bg_g bg_b bg_a outside, default none) filter (1 bilinear, 0 nearest) mip (level, 31 from the scale) |
| `dir_warp` | in, map | move each pixel along a direction by the brightness of map: intensity (1/256 of the image per unit of map), angle (turns), map_offset (brightness that moves nothing), tile_x tile_y (1 repeat, 0 repeat the edge) |
| `grad_warp` | in, map | move each pixel along the slope of map: intensity (default 1; 1/256 of the image per unit of slope in uv), aspect_x, aspect_y (default 1), tile_x tile_y (1 repeat, 0 keep the edge: no slope across it, edge value read) |
| `box_blur` | in | blur with a box in both axes reaching intensity pixels of a 256 pixel image each way: intensity aspect_x aspect_y (scale it per axis) tile_x tile_y (1 repeat, 0 repeat the edge) |
| `line_blur` | in | average along a line reaching intensity pixels of a 256 pixel image each way, by shearing the image, a box along the longer axis and shearing back: intensity aspect_x aspect_y (scale it per axis) angle (turns) tile_x tile_y (1 repeat, 0 repeat the edge) |
| `laplacian_sharpen` | in | add intensity times the difference to the four neighbours, limited to 0..1: intensity aspect_x aspect_y (scale it per axis) tile_x tile_y (1 repeat, 0 repeat the edge) |
| `pattern_program` | in (optional), in2 (optional), in3 (optional), in4 (optional), in5 (optional), in6 (optional), in7 (optional), in8 (optional), in9 (optional), in10 (optional), in11 (optional), in12 (optional), in13 (optional), in14 (optional), in15 (optional), in16 (optional), in17 (optional), in18 (optional), in19 (optional), in20 (optional), in21 (optional), in22 (optional), in23 (optional), in24 (optional), in25 (optional), in26 (optional), in27 (optional), in28 (optional), in29 (optional), in30 (optional), in31 (optional), in32 (optional), bg (optional) | run a pattern program: program="name" seed color(0/1); bg is an image the canvas starts from |

### Function-graph nodes

| Node | Inputs | Description and parameters |
|---|---|---|
| `f_out` | in | result of the function |
| `f_pin` | a | re-route a value link |
| `f_float` |  | constant float: v |
| `f_int` |  | constant integer: v |
| `f_bool` |  | constant boolean: v (0 or 1) |
| `f_string` |  | constant string: v |
| `f_vec2` | x (optional), y (optional) | float2 from parts (x y, linked parts win over the typed values) |
| `f_vec3` | x (optional), y (optional), z (optional) | float3 from parts |
| `f_vec4` | x (optional), y (optional), z (optional), w (optional) | float4 from parts |
| `f_ivec2` | x (optional), y (optional) | integer2 from parts |
| `f_ivec3` | x (optional), y (optional), z (optional) | integer3 from parts |
| `f_ivec4` | x (optional), y (optional), z (optional), w (optional) | integer4 from parts |
| `f_swizzle` | a | pick or reorder components: mask (xyzw or rgba, e.g. "zyx") |
| `f_neg` | a | negate |
| `f_abs` | a | absolute value |
| `f_sqrt` | a | square root (0 for negative input) |
| `f_exp` | a | e to the power a |
| `f_log` | a | natural logarithm (0 for input <= 0) |
| `f_log_floor` | a | natural logarithm, very low for input <= 0 (the engine of compiled graphs) |
| `f_log2` | a | base-2 logarithm (0 for input <= 0) |
| `f_sin` | a | sine (radians) |
| `f_cos` | a | cosine (radians) |
| `f_sin_poly` | a | sine by the polynomial of the engine of compiled graphs' fx-maps and node parameters |
| `f_cos_poly` | a | cosine by the polynomial of the engine of compiled graphs' fx-maps and node parameters |
| `f_sin_f32turns` | a | sine as a compiled graph's pixel processor has it on a GPU: the turn count truncated in single precision |
| `f_cos_f32turns` | a | cosine as a compiled graph's pixel processor has it on a GPU: the turn count truncated in single precision |
| `f_tan` | a | tangent (radians) |
| `f_asin` | a | inverse sine (input clamped to -1..1) |
| `f_acos` | a | inverse cosine (input clamped to -1..1) |
| `f_atan` | a | inverse tangent |
| `f_radians` | a | degrees to radians |
| `f_degrees` | a | radians to degrees |
| `f_floor` | a | round down |
| `f_ceil` | a | round up |
| `f_round` | a | round to nearest |
| `f_fract` | a | fractional part |
| `f_sign` | a | -1, 0 or 1 |
| `f_saturate` | a | clamp to 0..1 |
| `f_not` | a | boolean not |
| `f_length` | a | length of a vector |
| `f_normalize` | a | vector of length 1 (zero stays zero) |
| `f_add` | a, b | a + b |
| `f_sub` | a, b | a - b |
| `f_mul` | a, b | a * b |
| `f_div` | a, b | a / b (0 when b is 0; integer division truncates) |
| `f_div_large` | a, b | a / b, very large when b is 0 and a is not (the engine of compiled graphs) |
| `f_mod` | a, b | remainder of a / b with the sign of b (0 when b is 0) |
| `f_pow` | a, b | a to the power b |
| `f_min` | a, b | smaller value |
| `f_max` | a, b | larger value |
| `f_atan2` | a, b | angle of the point (b, a) |
| `f_atan2_approx` | a, b | angle of the point (b, a), by the approximation of the engine of compiled graphs |
| `f_step` | a, b | 0 when b < a, else 1 |
| `f_dot` | a, b | dot product |
| `f_cross` | a, b | cross product of two float3 |
| `f_distance` | a, b | distance between two points |
| `f_clamp` | a, b, c | clamp a between b (low) and c (high) |
| `f_lerp` | a, b, c | a + (b - a) * c |
| `f_smoothstep` | a, b, c | smooth 0..1 ramp of c between edges a and b |
| `f_eq` | a, b | a == b |
| `f_ne` | a, b | a != b |
| `f_lt` | a, b | a < b |
| `f_le` | a, b | a <= b |
| `f_gt` | a, b | a > b |
| `f_ge` | a, b | a >= b |
| `f_and` | a, b | boolean and |
| `f_or` | a, b | boolean or |
| `f_xor` | a, b | boolean exclusive or |
| `f_if` | cond, a, b | a when cond is true, else b (only the chosen branch runs) |
| `f_sequence` | a, b | run a, then b; returns b |
| `f_loop` | a, c, b (optional) | run a, then c again and again: until b is true, or `passes` times; returns 0 |
| `f_set` | a | store a in the variable `name`; returns a |
| `f_get` |  | read the variable `name` (0 until it is set); type: what it holds, for type checks (default float) |
| `f_random` | a (optional), b (optional), s (optional) | random float between a (default 0) and b (default 1); `seed` shifts the sequence; `lcg` draws from a 24-bit generator as compiled graphs do, by where the function runs (pixel, value, parameter, size, fx_luminance ... fx_image_index; see names::LCG_SITES and lcg_rand), with seed s and draw number k |
| `f_sample_gray` | a | brightness of input image `slot` (default in) at the position a (float2, 0..1); filter: bit 0 bilinear, else nearest, bit 1 keep the edge, else repeat (default bilinear, repeat) |
| `f_sample_color` | a | colour (float4) of input image `slot` at the position a; filter as for f_sample_gray |
| `f_image_avg` |  | average colour (float4) of input image `slot` |
| `f_image_min` |  | smallest channel values (float4) of input image `slot` |
| `f_image_max` |  | largest channel values (float4) of input image `slot` |
| `f_pos` |  | position being evaluated (float2, 0..1) |
| `f_size` |  | output size in pixels (float2) |
| `f_own_size` |  | size in pixels (float2) of the node whose parameter this computes; the output size elsewhere |
| `f_cell_size` |  | size of the cell a pattern program is walking, in uv (float; 1 elsewhere) |
| `f_cell_depth` |  | how many quadrants deep the cell of a pattern program is (float; 0 elsewhere) |
| `f_index` |  | instance / iteration number (integer) |
| `f_count` |  | number of instances / iterations (integer) |
| `f_seed` |  | random seed of the caller (integer) |
| `f_input` |  | input of this function: name, type (0 float, 1 int, 2 bool, 3 float2, 4 float3, 5 float4, 6 int2, 7 int3, 8 int4, 9 string), default v / x y z w |
| `f_graph_input` |  | value of an Input Value node of the graph the caller lives in: name, type, default v / x y z w |
| `f_to_float` | a | convert to float |
| `f_to_int` | a | convert to integer (drops the fraction) |
| `f_to_bool` | a | non-zero is true |
| `f_to_string` | a | text of a value |
| `f_concat` | a, b | join two strings |
| `f_call` | a (optional), b (optional), c (optional), d (optional) | run another function (`fn`); a b c d fill its inputs in the order of their node numbers |

### FX-map program nodes

| Node | Inputs | Description and parameters |
|---|---|---|
| `fxm_out` | in | start of a pattern program |
| `fxm_marker` |  | draw a shape: shape (0 circle 1 square 2 diamond 3 image) src x y w h rot value r g b softness rnd_pos rnd_scale rnd_rot rnd_val rnd_color |
| `fxm_quadrant` | q0 (optional), q1 (optional), q2 (optional), q3 (optional) | split the cell in four: scale off_x off_y p0..p3 (chance) v0..v3 (value) r0..r3 (rotation) rnd_pos rnd_scale rnd_rot rnd_val |
| `fxm_iterate` | body (optional) | repeat body: count dx dy dscale drot dval, or grid_x grid_y stagger; rnd_x rnd_y rnd_scale rnd_rot rnd_val |
| `fxm_switch` | a (optional), b (optional), c (optional), d (optional) | run one branch: mode (0 index, 1 random) index w0..w3 |

### Pattern-program nodes

| Node | Inputs | Description and parameters |
|---|---|---|
| `pp_out` | in | start of a pattern program; fn.init, then fn.background, fn.roughness, fn.opacity, fn.frame and fn.frame_offset run first |
| `pp_iterate` | first (optional), body (optional) | run first once, then body count times (count or fn.count); f_index is the iteration |
| `pp_switch` | a (optional), b (optional) | run b when cond (or fn.cond) is true, else a |
| `pp_quadrant` | q0 (optional), q1 (optional), q2 (optional), q3 (optional) | one cell, split in four for q0..q3: draws pattern (square, disc, paraboloid, bell, gaussian, thorn, pyramid, brick, gradation, waves, half_bell, ridged_bell, crescent, capsule, cone, gradation_offset, hemisphere), or with image=1 the input image_index, with luminance, branch_offset_x/y, offset_x/y, size_x/y, rotation, variation, max (each also fn.<name>) |
