//! Layout sizes go through `style`, so the zoom scales them; only hairlines stay px.

use std::fs;
use std::path::Path;

// Files whose px values are not layout sizes: the constants themselves, zoom math,
// saved window geometry.
const EXEMPT: [&str; 3] = ["style.rs", "zoom.rs", "session.rs"];
// One and two px lines (borders, outlines, separators) stay crisp at every zoom.
const HAIRLINES: [&str; 6] = [
    "px(0.)", "px(1.)", "px(2.)", "px(0.0)", "px(1.0)", "px(2.0)",
];

fn walk(dir: &Path, out: &mut Vec<String>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs")
            && !EXEMPT.contains(&p.file_name().unwrap().to_str().unwrap())
        {
            for (n, line) in fs::read_to_string(&p).unwrap().lines().enumerate() {
                let mut rest = line;
                while let Some(i) = rest.find("px(") {
                    let tail = &rest[i..];
                    // `.px(` and `input_px(` are methods, not the `px` constructor.
                    let boundary = i == 0 || {
                        let c = rest.as_bytes()[i - 1];
                        !(c.is_ascii_alphanumeric() || c == b'_' || c == b'.')
                    };
                    if boundary && !HAIRLINES.iter().any(|h| tail.starts_with(h)) {
                        out.push(format!("{}:{}: {}", p.display(), n + 1, line.trim()));
                        break;
                    }
                    rest = &rest[i + 3..];
                }
            }
        }
    }
}

#[test]
fn no_view_sizes_itself_in_pixels() {
    let mut hits = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut hits,
    );
    assert!(
        hits.is_empty(),
        "use a style constant (rems):\n{}",
        hits.join("\n")
    );
}
