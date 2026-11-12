# craft-geom

Plane geometry for photo editing, from LightCraft's `lightcraft-geom` at `a75497d6`. Image
coordinates are y-down; *normalised* coordinates map the full (uncropped, oriented) image to
`0..1` on both axes, so tools and settings stay resolution independent.

- `Point`, `Vec2`, `Rect`, `Affine` and `Orientation` (the eight Exif orientations).
- `CropGeometry`: a normalised crop rectangle in the rotated frame plus a straighten angle;
  `crop_fit_angle` and `max_inscribed_scale` keep a straightened crop inside the image.
- `Homography`: projective transforms from four point pairs (Upright, perspective, keystone).
- `drag_crop` and `CropHandle`: what dragging a crop handle or edge does, with aspect lock.
- `Real`: the scalar the coordinate formulas are generic over, `f64` or `Interval` (bounds on the results over a range of inputs, such as a block of output pixels), so one formula serves both.

No dependencies beyond `serde`. It is shared by `craft-develop-model` and the develop pipeline.
