fn main() {
    let a: Vec<String> = std::env::args().collect();
    let d = match a[1].as_str() { "hspice" => spicerack::codegen::Dialect::Hspice, "ltspice" => spicerack::codegen::Dialect::Ltspice, "xyce" => spicerack::codegen::Dialect::Xyce, "spectre" => spicerack::codegen::Dialect::Spectre, "vacask" => spicerack::codegen::Dialect::Vacask, _ => spicerack::codegen::Dialect::Ngspice };
    let src = spicerack::import::decode(&std::fs::read(&a[2]).unwrap());
    print!("{}", spicerack::import::dump(&src, d).unwrap());
}
