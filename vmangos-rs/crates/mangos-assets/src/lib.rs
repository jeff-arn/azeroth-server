//! LE-explicit parsers for extracted client asset data, byte- and
//! f32-exact with the C++ loaders: [`gridmap`] (`.map` terrain
//! tiles) and [`vmap`] (`.vmo`/`.vmtree`/`.vmtile` collision
//! geometry + BIH ray queries). All multi-byte reads are explicit
//! little-endian — no native-order struct reads (the C++ `fread`
//! hazard of §11).

pub mod gridmap;
pub mod vmap;
