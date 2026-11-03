//! The parser, one test per node tag: each names the manual clause it
//! reads, the tree derived by hand from that clause, the diagnostic codes
//! it expects, and how many checks it makes (rule 10 of the
//! compiler-construction skill).
//!
//! A tree prints as `Tag"head"[children]`; a `Word` as `w"text"`, a number
//! `n"text"`, a name `i"text"`, a string `s"text"`, an operator node as
//! `(op lhs rhs)`.
//!
//! Contents: none (tests only).

use crate::syntax::ast::{Ast, NONE, Node, Tag};
use crate::syntax::device::Primitive;
use crate::syntax::diag::Code;
use crate::syntax::lexicon::{HSPICE, LTSPICE, Lexicon, NGSPICE, SPECTRE, VACASK, XYCE};
use crate::syntax::statement::{Analysis, Command};
use crate::syntax::token::Tokens;
use crate::syntax::{parse, tokenize};

mod heads;

// ---- private ----

/// `src`'s top statements (the title dropped), one tree each, and the
/// diagnostic codes.
fn read(src: &str, lex: &'static Lexicon) -> (Vec<String>, Vec<Code>) {
    let t = tokenize(src, lex);
    let a = parse(&t, lex);
    let top = a
        .fields(a.root())
        .iter()
        .filter(|&&n| a.tag(Node(n)) != Tag::Title)
        .map(|&n| show(&a, &t, n))
        .collect();
    (top, a.diags().iter().map(|d| d.code).collect())
}

fn show(a: &Ast, t: &Tokens, n: u32) -> String {
    if n == NONE {
        return "-".into();
    }
    let node = Node(n);
    let head = t.text(a.main_token(node));
    let list = |items: &[u32]| {
        items
            .iter()
            .map(|&k| show(a, t, k))
            .collect::<Vec<_>>()
            .join(" ")
    };
    match a.tag(node) {
        Tag::Word => format!("w{:?}", a.name(crate::syntax::ast::Name(a.rhs(node)))),
        Tag::Num => format!("n{head:?}"),
        Tag::Ident => format!("i{:?}", a.name(crate::syntax::ast::Name(a.lhs(node)))),
        Tag::Str => format!("s{head:?}"),
        Tag::Unary => format!("({head} {})", show(a, t, a.lhs(node))),
        Tag::Binary => format!(
            "({head} {} {})",
            show(a, t, a.lhs(node)),
            show(a, t, a.rhs(node))
        ),
        Tag::Ternary => {
            let at = a.rhs(node);
            let (x, y) = (a.extra(at), a.extra(at + 1));
            format!(
                "(? {} {} {})",
                show(a, t, a.lhs(node)),
                show(a, t, x),
                show(a, t, y)
            )
        }
        Tag::Paren | Tag::Delimited => {
            format!("{:?}{head:?}[{}]", a.tag(node), show(a, t, a.lhs(node)))
        }
        Tag::Assign => format!("{}={}", show(a, t, a.lhs(node)), show(a, t, a.rhs(node))),
        Tag::Call | Tag::Probe | Tag::Group | Tag::Vector | Tag::List => {
            let items = a.list(a.rhs(node) + 1);
            format!("{:?}{head:?}[{}]", a.tag(node), list(items))
        }
        Tag::Index => {
            let at = a.rhs(node);
            format!(
                "Index[{} {}]",
                show(a, t, a.lhs(node)),
                show(a, t, a.extra(at))
            )
        }
        Tag::Seq => format!("Seq[{}]", list(a.list(a.lhs(node)))),
        Tag::Bad | Tag::Verbatim | Tag::Line => format!("{:?}{head:?}", a.tag(node)),
        tag if tag.has_header() => {
            let h = a.header(node);
            let code = match tag {
                Tag::Instance => Primitive::from_u32(h.code).map(|p| format!("{p:?}")),
                Tag::Analysis => Analysis::from_u32(h.code).map(|k| format!("{k:?}")),
                Tag::Command => Command::from_u32(h.code).map(|c| format!("{c:?}")),
                _ => None,
            };
            let mut s = format!(
                "{tag:?}{head:?}<{} {} {}|{}>[{}]",
                code.unwrap_or("-".into()),
                show(a, t, h.name),
                show(a, t, h.master),
                list(h.nodes),
                list(a.fields(node))
            );
            if a.rhs(node) != NONE {
                s.push_str(&format!("{{{}}}", list(a.body(node))));
            }
            s
        }
        tag => {
            let mut s = format!("{tag:?}{head:?}[{}]", list(a.fields(node)));
            if a.rhs(node) != NONE {
                s.push_str(&format!("{{{}}}", list(a.body(node))));
                if let Some(nx) = a.next(node) {
                    s.push_str(&format!(" -> {}", show(a, t, nx.0)));
                }
            }
            s
        }
    }
}

#[test]
fn title_and_end() {
    // ngspice manual §1.2: line 1 is the title; §1.7: `.end` is a card.
    // Expect: Title then End. Codes: none. Checks: 2.
    let t = tokenize("t\n.end\n", &NGSPICE);
    let a = parse(&t, &NGSPICE);
    let tags: Vec<Tag> = a.fields(a.root()).iter().map(|&n| a.tag(Node(n))).collect();
    assert_eq!(tags, [Tag::Title, Tag::End]);
    assert!(a.diags().is_empty());
}

#[test]
fn verbatim_for_a_headless_line() {
    // ngspice manual §2: a card starts with a letter or a dot; `5 a b` has
    // neither. Expect: Verbatim with P0001. Checks: 2.
    let (top, codes) = read("t\n5 a b\n", &NGSPICE);
    assert_eq!(top, ["Verbatim\"5\""]);
    assert_eq!(codes, [Code::P0001]);
}

#[test]
fn include_and_lib_call() {
    // ngspice manual §2.6 `.include filename`, §2.7 `.lib filename libname`:
    // a path is one blank-free chunk. Checks: 2.
    let (top, codes) = read("t\n.include a/b(1).sp\n.lib \"m.lib\" tt\n", &NGSPICE);
    assert_eq!(
        top,
        [
            "Include\".include\"[w\"a/b(1).sp\"]",
            "LibCall\".lib\"[s\"\\\"m.lib\\\"\" w\"tt\"]"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn subckt_with_params() {
    // ngspice manual §2.5: `.SUBCKT subnam N1 <N2 ...> params: p=v`, ended by
    // `.ENDS`. Expect: ports and the default as fields, the card as body,
    // the `.ends` as closer. Checks: 2.
    let (top, codes) = read(
        "t\n.subckt s a b params: w=1\nR1 a b 1k\n.ends s\n",
        &NGSPICE,
    );
    assert_eq!(
        top,
        [
            "Subckt\".subckt\"[w\"s\" w\"a\" w\"b\" w\"params:\" w\"w\"=n\"1\"]{Instance\"R1\"<Resistor w\"R1\" -|w\"a\" w\"b\">[w\"1k\"]}"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn section_is_lib_with_one_word() {
    // ngspice manual §2.7: `.lib libname` ... `.endl` defines a section; a
    // second section head inside one does not nest (P0009). Checks: 2.
    let (top, codes) = read("t\n.lib tt\n.lib ss\n.param a=1\n.endl\n", &NGSPICE);
    assert_eq!(
        top,
        ["Section\".lib\"[w\"tt\"]{Verbatim\".lib\" Param\".param\"[w\"a\"=n\"1\"]}"]
    );
    assert_eq!(codes, [Code::P0009, Code::P0001]);
}

#[test]
fn library_holds_sections() {
    // Spectre Reference 19.1 p.498: `library name` ... `endlibrary` holds
    // `section name` ... `endsection`. Checks: 2.
    let (top, codes) = read(
        "t\nlibrary L\nsection tt\nparameters a=1\nendsection tt\nendlibrary L\n",
        &SPECTRE,
    );
    assert_eq!(
        top,
        [
            "Library\"library\"[w\"L\"]{Section\"section\"[w\"tt\"]{Param\"parameters\"[w\"a\"=n\"1\"]}}"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn if_else_chain() {
    // ngspice manual §2.15: `.if (cond)` ... `.elseif (cond)` ... `.else`
    // ... `.endif`. Expect: If -> If -> Else, the conditions as expression
    // trees. Checks: 2.
    let (top, codes) = read(
        "t\n.if (a == 1)\nR1 a 0 1\n.elseif (b)\n.else\n.endif\n",
        &NGSPICE,
    );
    assert_eq!(
        top,
        [
            "If\".if\"[Paren\"(\"[(== i\"a\" n\"1\")]]{Instance\"R1\"<Resistor w\"R1\" -|w\"a\" w\"0\">[w\"1\"]} -> If\".elseif\"[Paren\"(\"[i\"b\"]]{} -> Else\".else\"[]{}"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn spectre_if_else_and_unbraced_branch() {
    // Spectre Reference 19.1 p.491: `if (c) statement [else statement]`, a
    // statement a braced list or one card. Checks: 2.
    let (top, codes) = read(
        "t\nif (m == 0) {\nr0 (a b) resistor r=1\n} else\nr1 (a b) resistor r=2\n",
        &SPECTRE,
    );
    assert_eq!(
        top,
        [
            "If\"if\"[Paren\"(\"[(== i\"m\" n\"0\")]]{Instance\"r0\"<Resistor w\"r0\" w\"resistor\"|w\"a\" w\"b\">[w\"r\"=n\"1\"]} -> Else\"else\"[]{Instance\"r1\"<Resistor w\"r1\" w\"resistor\"|w\"a\" w\"b\">[w\"r\"=n\"2\"]}"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn spectre_condition_over_two_lines() {
    // Spectre UG p.59: a statement ends at the line's end; inside open
    // parentheses it does not (the condition of `if (a &&` ⏎ `b) {`).
    // Checks: 2.
    let (top, codes) = read("t\nif (a &&\n b) {\n}\n", &SPECTRE);
    assert_eq!(top, ["If\"if\"[Paren\"(\"[(&& i\"a\" i\"b\")]]{}"]);
    assert!(codes.is_empty());
}

#[test]
fn control_reads_analysis_commands_only() {
    // ngspice manual §13.1: `.control` ... `.endc` is a script; its `tran`
    // runs the analysis (read as `.tran`), the rest are lines. Checks: 2.
    let (top, codes) = read("t\n.control\ntran 1n 10n\nrun\n.endc\n", &NGSPICE);
    assert_eq!(
        top,
        ["Control\".control\"[]{Analysis\"tran\"<Tran - -|>[w\"1n\" w\"10n\"] Line\"run\"}"]
    );
    assert!(codes.is_empty());
}

#[test]
fn region_reads_spice_inside_spectre() {
    // Spectre UG p.53: `simulator lang=spice` holds until `simulator
    // lang=spectre`; the region is SPICE. Checks: 2.
    let (top, codes) = read(
        "t\nsimulator lang=spice\n.param a=1\nsimulator lang=spectre\n",
        &SPECTRE,
    );
    assert_eq!(
        top,
        ["Region\"simulator\"[w\"lang\"=i\"spice\"]{Param\".param\"[w\"a\"=n\"1\"]}"]
    );
    assert!(codes.is_empty());
}

#[test]
fn data_block_is_lines() {
    // HSPICE UG `.DATA` ... `.ENDDATA`: rows of numbers, kept as lines.
    // Checks: 2.
    let (top, codes) = read("t\n.data d\n1 2\n.enddata\n", &NGSPICE);
    assert_eq!(top, ["Data\".data\"[w\"d\"]{Line\"1\"}"]);
    assert!(codes.is_empty());
}

#[test]
fn param_values_run_across_blanks_to_the_next_key() {
    // ngspice manual §2.4.1: `.param a = b + c`; sky130's `a = '-1*x' + b =
    // 2`: an operator before `key =` separates. Checks: 2.
    let (top, codes) = read("t\n.param r = a + 1 s='-1*x' + t = 2\n", &NGSPICE);
    assert_eq!(
        top,
        [
            "Param\".param\"[w\"r\"=(+ i\"a\" n\"1\") w\"s\"=Delimited\"'\"[(* (- n\"1\") i\"x\")] w\"+\" w\"t\"=n\"2\"]"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn func_definition() {
    // ngspice manual §2.4.2: `.func name(args) {body}`. Checks: 2.
    let (top, codes) = read("t\n.func f(x) {x*2}\n", &NGSPICE);
    assert_eq!(
        top,
        ["Func\".func\"[Group\"f\"[w\"x\"] Delimited\"{\"[(* i\"x\" n\"2\")]]"]
    );
    assert!(codes.is_empty());
}

#[test]
fn spectre_real_function_over_three_lines() {
    // Spectre UG p.92: `real f(real a) { return expr }`, the brace on the
    // next line. Checks: 2.
    let (top, codes) = read("t\nreal f(real a)\n{\nreturn a*2;\n}\n", &SPECTRE);
    assert_eq!(
        top,
        ["Func\"real\"[w\"f\" w\"real\" w\"a\"]{Card\"return\"[(* i\"a\" n\"2\")]}"]
    );
    assert!(codes.is_empty());
}

#[test]
fn model_matrix_is_a_seq() {
    // ngspice manual §2.3 `.model name type (p=v ...)`; a CPL model's matrix
    // `c=1e-10 -6e-11` is two numbers (spice.peg `MVal`). Checks: 2.
    let (top, codes) = read("t\n.model m cpl (c=1e-10 -6e-11 r=1)\n", &NGSPICE);
    assert_eq!(
        top,
        ["Model\".model\"[w\"m\" w\"cpl\" w\"c\"=Seq[w\"1e-10\" w\"-6e-11\"] w\"r\"=w\"1\"]"]
    );
    assert!(codes.is_empty());
}

#[test]
fn spectre_model_group_bins() {
    // Spectre UG p.108: `model name master {` bins `1: p=v` `}`. Checks: 2.
    let (top, codes) = read("t\nmodel n bsim4 {\n1: lmin=1\n}\n", &SPECTRE);
    assert_eq!(
        top,
        ["Model\"model\"[w\"n\" w\"bsim4\"]{Card\"1\"[w\":\" w\"lmin\"=n\"1\"]}"]
    );
    assert!(codes.is_empty());
}

#[test]
fn testbench_cards() {
    // ngspice manual §11.1 (`.options`), §11.3 (`.tran`), §15.6 (`.save`,
    // `.print`), §15.4 (`.meas`), §11.2 (`.ic`, `.nodeset`), §2.11 (`.temp`),
    // §2.8 (`.global`). Checks: 2.
    let src = "t\n.options reltol=1e-3 klu\n.tran 1n 10n\n.save v(a)\n.print tran v(a)\n.meas tran x find v(a) at=1n\n.ic v(a)=1\n.nodeset v(b)=0\n.temp 27\n.global vdd\n";
    let (top, codes) = read(src, &NGSPICE);
    assert_eq!(
        top,
        [
            "Options\".options\"<- - -|>[w\"reltol\"=n\"1e-3\" w\"klu\"]",
            "Analysis\".tran\"<Tran - -|>[w\"1n\" w\"10n\"]",
            "Save\".save\"[Group\"v\"[w\"a\"]]",
            "Print\".print\"[w\"tran\" Group\"v\"[w\"a\"]]",
            "Measure\".meas\"[w\"tran\" w\"x\" w\"find\" Group\"v\"[w\"a\"] w\"at\"=n\"1n\"]",
            "Ic\".ic\"[Group\"v\"[w\"a\"]=n\"1\"]",
            "Nodeset\".nodeset\"[Group\"v\"[w\"b\"]=n\"0\"]",
            "Temp\".temp\"[w\"27\"]",
            "Global\".global\"[w\"vdd\"]"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn step_card() {
    // LTspice SCR `.STEP param R 1k 10k 1k`. Checks: 2.
    let (top, codes) = read("t\n.step param r 1k 10k 1k\n", &LTSPICE);
    assert_eq!(
        top,
        ["Step\".step\"[w\"param\" w\"r\" w\"1k\" w\"10k\" w\"1k\"]"]
    );
    assert!(codes.is_empty());
}

#[test]
fn statistics_process_vary() {
    // Spectre Reference 19.1 pp.175-179: `statistics { process { vary p
    // dist=gauss std=1 } }`, on one line. Checks: 2.
    let (top, codes) = read(
        "t\nstatistics { process { vary p dist=gauss std=1 } }\n",
        &SPECTRE,
    );
    assert_eq!(
        top,
        [
            "Statistics\"statistics\"[]{Process\"process\"[]{Card\"vary\"[w\"p\" w\"dist\"=i\"gauss\" w\"std\"=n\"1\"]}}"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn card_for_other_directives() {
    // ngspice manual §11.3: `.mc`, `.pss` and other analyses the IR does not
    // model keep their fields. Checks: 2.
    let (top, codes) = read("t\n.mc 10 tran\n", &NGSPICE);
    assert_eq!(top, ["Card\".mc\"[w\"10\" w\"tran\"]"]);
    assert!(codes.is_empty());
}

#[test]
fn words_join_gapless_tokens() {
    // ngspice manual §2.1: node names are any non-separator characters
    // (`net-1`, `vdd!`, `n<3>`); INPgetTok reads `10%3` as one token. Checks: 2.
    let (top, codes) = read("t\nR1 net-1 vdd! 10%3\n", &NGSPICE);
    assert_eq!(
        top,
        ["Instance\"R1\"<Resistor w\"R1\" -|w\"net-1\" w\"vdd!\">[w\"10%3\"]"]
    );
    assert!(codes.is_empty());
}

#[test]
fn group_and_probe() {
    // ngspice manual §4.1 waveforms `SIN(VO VA FREQ)`; §2.8.5 `v(a,b)` in an
    // expression is a probe. Checks: 2.
    let (top, codes) = read("t\nV1 a 0 sin(0 1 1k)\nB1 a 0 v={v(a,b)*2}\n", &NGSPICE);
    assert_eq!(
        top,
        [
            "Instance\"V1\"<VoltageSource w\"V1\" -|w\"a\" w\"0\">[Group\"sin\"[w\"0\" w\"1\" w\"1k\"]]",
            "Instance\"B1\"<Behavioural w\"B1\" -|w\"a\" w\"0\">[w\"v\"=Delimited\"{\"[(* Probe\"v\"[w\"a\" w\"b\"] n\"2\")]]"
        ]
    );
    assert!(codes.is_empty());
}

#[test]
fn element_comma_vector_is_a_seq() {
    // ngspice manual §3.3.2 `ic=0,-5`: an instance's comma vector. Checks: 2.
    let (top, codes) = read("t\nC1 a 0 1p ic=0,-5\n", &NGSPICE);
    assert_eq!(
        top,
        ["Instance\"C1\"<Capacitor w\"C1\" -|w\"a\" w\"0\">[w\"1p\" w\"ic\"=Seq[n\"0\" w\"-5\"]]"]
    );
    assert!(codes.is_empty());
}

#[test]
fn hspice_string_is_an_expression() {
    // HSPICE UG08 T2, X1: `"expr"` delimits an expression; a broken one is
    // P0002. Checks: 2.
    let (top, codes) = read("t\n.param a=\"1+2\" b=\"1+\"\n", &HSPICE);
    assert_eq!(
        top,
        ["Param\".param\"[w\"a\"=Delimited\"\\\"1+2\\\"\"[-] w\"b\"=Bad\"\\\"1+\\\"\"]"]
    );
    assert_eq!(codes, [Code::P0002]);
}

#[test]
fn operators_follow_each_dialects_table() {
    // ngspice §2.8.5: unary minus takes a power (`-2^2` is `-(2^2)`); LTspice
    // CER: the sign binds tighter than `**`; Xyce RG Table 2-32: `**` groups
    // to the right; Spectre UG p.87: `c ? a : b`. Checks: 4.
    assert_eq!(
        read("t\n.param a={-2^2}\n", &NGSPICE).0,
        ["Param\".param\"[w\"a\"=Delimited\"{\"[(- (^ n\"2\" n\"2\"))]]"]
    );
    assert_eq!(
        read("t\n.param a={-2**2}\n", &LTSPICE).0,
        ["Param\".param\"[w\"a\"=Delimited\"{\"[(** (- n\"2\") n\"2\")]]"]
    );
    assert_eq!(
        read("t\n.param a={2**3**2}\n", &XYCE).0,
        ["Param\".param\"[w\"a\"=Delimited\"{\"[(** n\"2\" (** n\"3\" n\"2\"))]]"]
    );
    assert_eq!(
        read("t\nparameters a=b>1 ? 1 : 2\n", &SPECTRE).0,
        ["Param\"parameters\"[w\"a\"=(? (> i\"b\" n\"1\") n\"1\" n\"2\")]"]
    );
}

#[test]
fn call_ident_str() {
    // Spectre UG p.87: function calls and names; §2.5 strings. Checks: 2.
    let (top, codes) = read("t\nparameters a=max(b, 1) s=\"x\"\n", &SPECTRE);
    assert_eq!(
        top,
        ["Param\"parameters\"[w\"a\"=Call\"max\"[i\"b\" n\"1\"] w\"s\"=s\"\\\"x\\\"\"]"]
    );
    assert!(codes.is_empty());
}

#[test]
fn vector_list_index() {
    // Spectre §5: `[1p 2p]` blank-separated; VACASK P740-P781: `[a, b]`,
    // `{a, b}`, `x[i]`. Checks: 3.
    assert_eq!(
        read("t\nc1 (a b) capacitor c=[1p 2p]\n", &SPECTRE).0,
        [
            "Instance\"c1\"<Capacitor w\"c1\" w\"capacitor\"|w\"a\" w\"b\">[w\"c\"=Vector\"[\"[n\"1p\" n\"2p\"]]"
        ]
    );
    assert_eq!(
        read("t\nparameters v=[1, 2] l={1, 2} x=v[1]\n", &VACASK).0,
        [
            "Param\"parameters\"[w\"v\"=Vector\"[\"[n\"1\" n\"2\"] w\"l\"=List\"{\"[n\"1\" n\"2\"] w\"x\"=Index[i\"v\" n\"1\"]]"
        ]
    );
    assert!(read("t\nparameters x=v[1]\n", &VACASK).1.is_empty());
}

#[test]
fn bad_expression_and_unclosed_delimiter() {
    // ngspice manual §2.1: `{expr}` must hold one expression and close on
    // its card: `{1 2}` is P0002, `{1+` is P0008. Checks: 2.
    let (top, codes) = read("t\n.param a={1 2} b={1+\n", &NGSPICE);
    assert_eq!(
        top,
        ["Param\".param\"[w\"a\"=Delimited\"{\"[Bad\"1\"] w\"b\"=Bad\"{\"]"]
    );
    assert_eq!(codes, [Code::P0002, Code::P0008]);
}

#[test]
fn nan_number_and_invalid_byte() {
    // Spectre §2.4: `1k5` is no number (P0005); a control byte is no token
    // (P0007), so `b=` has no value (P0002). Checks: 1.
    let (_, codes) = read("t\nparameters a=1k5 b=\x01\n", &SPECTRE);
    assert_eq!(codes, [Code::P0005, Code::P0007, Code::P0002]);
}

#[test]
fn unclosed_and_stray_blocks() {
    // ngspice manual §2.5: `.subckt` needs its `.ends` (P0003); an `.endl`
    // with no `.lib` open is P0004. Checks: 1.
    let (_, codes) = read("t\n.endl\n.subckt s a\n", &NGSPICE);
    assert_eq!(codes, [Code::P0004, Code::P0003]);
}
