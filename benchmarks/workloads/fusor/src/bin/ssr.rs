use std::time::Instant;
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let count = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(1000);
    if args.get(2).is_some_and(|arg| arg == "html") {
        print!("{}", fusor_bench_workload::server_render(count)?);
        return Ok(());
    }
    for _ in 0..3 {
        std::hint::black_box(fusor_bench_workload::server_render(count)?);
    }
    let mut times = vec![];
    let mut bytes = 0;
    for _ in 0..15 {
        let start = Instant::now();
        let html = fusor_bench_workload::server_render(count)?;
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        bytes = html.len();
        std::hint::black_box(html);
    }
    println!("{{\"framework\":\"fusor\",\"n\":{count},\"bytes\":{bytes},\"samples\":{times:?}}}");
    Ok(())
}
