use jka_assets::source1::parse_source1_skeleton;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let mdl = std::fs::read(&path).unwrap();
    let skeleton = parse_source1_skeleton(&mdl).unwrap();
    for (i, b) in skeleton.bones.iter().enumerate() {
        println!("{i:3} {:<40} parent={:?} pos={:?}", b.name, b.parent, b.bind_position);
    }
}
