#[macro_use]
extern crate bencher;

use bencher::Bencher;
use indoc::indoc;
use std::io::{self, Cursor};
use clarus::binhex::archive::BinHexArchive;

const BINHEX_SMALL: &[u8] = indoc! {br#"
            (This file must be converted with BinHex 4.0)
            :$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&
            dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!
            YN!8SI!:"#
        };

const BINHEX_LARGE: &[u8] = include_bytes!("stuffit-expander-4.hqx");

fn extract_small(bench: &mut Bencher) {
    let mut data_sink = io::sink();
    let mut rsrc_sink = io::sink();

    bench.iter(|| {
        BinHexArchive::new(Cursor::new(BINHEX_SMALL)).extract(&mut data_sink, &mut rsrc_sink)
            .expect("Failed to extract archive");
    });
}

fn extract_large(bench: &mut Bencher) {
    let mut data_sink = io::sink();
    let mut rsrc_sink = io::sink();

    bench.iter(|| {
        BinHexArchive::new(Cursor::new(BINHEX_LARGE)).extract(&mut data_sink, &mut rsrc_sink)
            .expect("Failed to extract archive");
    });
}

benchmark_group!(benches, extract_small, extract_large);
benchmark_main!(benches);