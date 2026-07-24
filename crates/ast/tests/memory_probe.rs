//! Manual memory checks for AST test fixtures.
//!
//! ```text
//! cargo test -p ast --test memory_probe -- --ignored --nocapture
//! ```
//!
//! Run with `--test-threads=1` if the machine is tight on RAM and many integration
//! tests are executing in parallel.

mod support;

const MAX_RSS_DELTA_BYTES: usize = 200 * 1024 * 1024;

#[test]
#[ignore = "manual: cargo test -p ast --test memory_probe -- --ignored --nocapture"]
fn marker_fixture_reuse_stays_bounded() {
    let rss_before = support::process_rss_bytes();

    let snippet = "\
Type is Primitive(width BigInt, signed Bool) or Record(name String, fields List(NamedTy))
NamedTy has name String, ty Type
List(x) has pointer Pointer(x), length BigInt
BigInt is in 0...1000
String has bytes List(Byte)
Byte is in 0...255
Bool is True or False
Pointer(x) has addr BigInt

f(x Type) Bool := when x is
    Record(_, fields) then True
    else False
";

    for _ in 0..12 {
        let typed = support::transform_with_marker_package(snippet);
        let _ = typed.hover_at(snippet, 0, 0);
    }

    let rss_after = support::process_rss_bytes();
    let delta = rss_after.saturating_sub(rss_before);

    println!("rss before       = {rss_before} bytes");
    println!("rss after 12x    = {rss_after} bytes");
    println!("delta            = {delta} bytes");

    if rss_before > 0 && rss_after > 0 {
        assert!(
            delta < MAX_RSS_DELTA_BYTES,
            "12 transforms with shared marker fixture grew RSS by {delta} bytes \
             (cap {MAX_RSS_DELTA_BYTES}); use cached fixtures and avoid full eval transform per test"
        );
    } else {
        eprintln!("skip RSS assertion: process_rss_bytes unavailable on this platform");
    }
}
