//! Production-fixture asset gate: the six start-area map/vmap
//! checks against the real extracted data tree. `#[ignore]`d +
//! gated on `MANGOS_DATADIR` (a datadir containing `maps/`,
//! `vmaps/`, `mmaps/`, `5875/`).

use mangos_world_data::DataDir;
use mangos_world_data::assets_check::check_start_area_assets;

#[test]
#[ignore = "requires MANGOS_DATADIR pointing at the production fixture datadir"]
fn start_area_assets_exist_in_fixtures() {
    let datadir = std::env::var("MANGOS_DATADIR").expect("MANGOS_DATADIR must be set");
    let dir = DataDir::new(datadir);
    check_start_area_assets(&dir).expect("all six start areas present with valid magics");
}
