use dashmap::DashMap;
fn main() {
    let map = DashMap::new();
    let entry = map.entry(1).or_insert(1);
    println!("{}", map.len());
}
