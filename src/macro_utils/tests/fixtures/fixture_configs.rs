use macro_utils::BenchConfig;
#[allow(dead_code)]
#[derive(BenchConfig)]
pub struct MConfig {
    name: String,
    threads: usize,
    time: usize,
    #[matrix]
    b: usize,
    #[matrix]
    c: usize,
    d: usize,
}

pub trait ConfigImpl {
    fn name(&self) -> &String;
    fn thread(&self) -> usize;
    fn bench_sec(&self) -> usize;
}

fn main() {}
