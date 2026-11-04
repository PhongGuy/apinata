use crate::support;

use apinata_app::command::Target;
use apinata_app::snapshot::SnapshotCache;
use apinata_core::Collection;
use std::sync::Arc;
use std::time::Instant;
use support::{ROOT, folder, ip, request, write_tree};

#[test]
#[ignore = "measurement; run with --release -- --ignored --nocapture"]
fn snapshot_cost_on_two_thousand_requests() {
    let mut files: Vec<(String, String)> = vec![("opencollection.yml".into(), ROOT.into())];
    for f in 0..40 {
        files.push((format!("f{f}/folder.yml"), folder(&format!("Folder {f}"))));
        for r in 0..50 {
            files.push((
                format!("f{f}/r{r}.yml"),
                request(&format!("R {f} {r}"), "http://x/{{host}}/items?q=1"),
            ));
        }
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let dir = write_tree(&refs);

    let started = Instant::now();
    let mut c = Collection::open(dir.path()).unwrap();
    let open = started.elapsed();

    let mut cache = SnapshotCache::new();
    let started = Instant::now();
    let cold = cache.build(&c);
    let cold_time = started.elapsed();

    // One keystroke: edit one request, touch it, rebuild; averaged over 100 rounds.
    let target = ip("f7/r7.yml");
    let mut warm = cold.clone();
    let started = Instant::now();
    for i in 0..100 {
        c.request_mut(&target)
            .unwrap()
            .set_url(format!("http://x/{i}"));
        cache.touch(&Target::Request(target.clone()));
        warm = cache.build(&c);
    }
    let warm_time = started.elapsed() / 100;
    let shared = cold
        .requests
        .iter()
        .filter(|(k, v)| Arc::ptr_eq(v, &warm.requests[*k]))
        .count();

    println!(
        "requests: {}, rows: {}",
        warm.requests.len(),
        warm.rows.len()
    );
    println!(
        "open: {open:?}, cold build: {cold_time:?}, warm build: {warm_time:?}, shared: {shared}"
    );
    assert_eq!(warm.requests.len(), 2000);
    assert_eq!(shared, 1999);
}
