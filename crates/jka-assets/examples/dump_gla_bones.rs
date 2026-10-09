use jka_assets::ghoul2::parse_gla;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let gla = parse_gla(&std::fs::read(&path).unwrap()).unwrap();
    for (i, b) in gla.skeleton.iter().enumerate() {
        let origin = [b.base_pose[0][3], b.base_pose[1][3], b.base_pose[2][3]];
        println!("{i:3} {:<16} parent={:<4} pos={:?}", b.name, b.parent, origin);
    }
}
