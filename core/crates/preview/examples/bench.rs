//! Render timings for docs/measurements.md:
//! `cargo run --release -p azulejo-preview --example bench`.

use std::time::Instant;

fn main() {
    let dir = tempfile::tempdir().unwrap();

    let section = "## Section heading\n\nA paragraph with *emphasis*, **strong**, `code` and a [link](other.md#x).\nIt wraps over a second line to look like real prose in a plan or a README file.\n\n- [x] done item\n- [ ] open item with `inline` code\n\n| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |\n\n```rust\nfn main() {\n    let v: Vec<u32> = (0..10).map(|x| x * 2).collect();\n    println!(\"{v:?}\");\n}\n```\n\n";
    let md = section.repeat(1024 * 1024 / section.len() + 1);
    let md_path = dir.path().join("big.md");
    std::fs::write(&md_path, &md).unwrap();

    let small = section.repeat(20);
    let small_path = dir.path().join("plan.md");
    std::fs::write(&small_path, &small).unwrap();

    let block = include_str!("../tests/fixtures/sample.rs");
    let lines = block.lines().count();
    let code = block.repeat(10_000 / lines + 1);
    let code_path = dir.path().join("big.rs");
    std::fs::write(&code_path, &code).unwrap();

    // Warm the syntax set and theme CSS once, as a running core would.
    let t = Instant::now();
    azulejo_preview::render(&dir.path().join("warm.rs"));
    std::fs::write(dir.path().join("warm.rs"), "fn x() {}\n").unwrap();
    azulejo_preview::render(&dir.path().join("warm.rs"));
    println!("first render (loads syntaxes): {:?}", t.elapsed());

    for (name, path, size, count) in [
        (
            "markdown (plan-sized)",
            &small_path,
            small.len(),
            small.lines().count(),
        ),
        ("markdown", &md_path, md.len(), md.lines().count()),
        ("rust", &code_path, code.len(), code.lines().count()),
    ] {
        let mut times = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            let r = azulejo_preview::render(path);
            times.push(t.elapsed());
            assert!(r.html.is_some());
        }
        times.sort();
        println!(
            "{name}: {:.2} MB, {count} lines → median {:?}, min {:?}, max {:?}",
            size as f64 / (1024.0 * 1024.0),
            times[2],
            times[0],
            times[4]
        );
    }
}
