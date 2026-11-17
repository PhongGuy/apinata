/// About 10 KB of text that expands to 64 million values with only about 1,100 alias jumps, so
/// yaml_serde's jump-per-event cap never trips.
pub fn hostile_yaml() -> String {
    let leaves = vec!["x"; 1000].join(", ");
    let a = vec!["*a"; 1000].join(", ");
    let b = vec!["*b"; 64].join(", ");
    format!("a: &a [{leaves}]\nb: &b [{a}]\nc: [{b}]\n")
}
