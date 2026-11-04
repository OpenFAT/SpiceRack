fn main() {
    let a: Vec<String> = std::env::args().collect();
    let d = match a[1].as_str() {
        "hspice" => spicerack::syntax::Dialect::Hspice,
        "ltspice" => spicerack::syntax::Dialect::Ltspice,
        "xyce" => spicerack::syntax::Dialect::Xyce,
        "spectre" => spicerack::syntax::Dialect::Spectre,
        "vacask" => spicerack::syntax::Dialect::Vacask,
        _ => spicerack::syntax::Dialect::Ngspice,
    };
    let src = spicerack::import::decode(&std::fs::read(&a[2]).unwrap());
    print!("{}", spicerack::import::dump(&src, d).unwrap());
}
