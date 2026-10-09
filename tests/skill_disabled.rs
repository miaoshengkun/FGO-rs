use fgo_rs::vision::Vision;
use std::path::PathBuf;

#[test]
fn saber_disabled_skill_template_is_loaded() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    assert!(vision.has_template("skill_cd_2_1_disabled"));
    assert!(vision.has_template("skill_cd_1_3_available"));
}
