use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const OPCODES_JSON: &str = "gb-opcodes/Opcodes.json";
/// The number of T-states in an M-cycle. The JSON counts the former, the crate the latter.
const T_STATES_PER_M_CYCLE: u64 = 4;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed={OPCODES_JSON}");

    let json = std::fs::read_to_string(OPCODES_JSON).expect("failed to read the opcodes JSON");
    let json: Value = serde_json::from_str(&json).expect("failed to parse the opcodes JSON");
    let unprefixed = read_table(&json, "unprefixed");
    let prefixed = read_table(&json, "cbprefixed");
    check_names(&unprefixed, &prefixed);

    let mut out = String::new();
    write_constants(&mut out, &unprefixed, "u8", |opcode| {
        format!("0x{opcode:02X}")
    });
    // a prefixed opcode is the two bytes it is encoded as, read as a little-endian word
    write_constants(&mut out, &prefixed, "u16", |opcode| {
        format!("0x{opcode:02X}CB")
    });
    write_table(&mut out, "INSTRUCTION_LEN", "u8", &unprefixed, |instr| {
        instr.len.to_string()
    });
    write_table(&mut out, "MNEMONIC", "&str", &unprefixed, |instr| {
        format!("{:?}", instr.mnemonic)
    });
    write_table(&mut out, "CYCLES", "Cycles", &unprefixed, |instr| {
        format!(
            "Cycles {{ min: {}, max: {} }}",
            instr.cycles.0, instr.cycles.1
        )
    });
    write_table(&mut out, "PREFIXED_CYCLES", "Cycles", &prefixed, |instr| {
        format!(
            "Cycles {{ min: {}, max: {} }}",
            instr.cycles.0, instr.cycles.1
        )
    });

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is not set"));
    std::fs::write(Path::new(&out_dir).join("opcodes.rs"), out)
        .expect("failed to write the generated opcodes");
}

struct Instr {
    mnemonic: String,
    /// The name of the instruction's constant.
    name: String,
    /// The instruction as it is written in assembly.
    asm: String,
    /// The encoded length in bytes, prefix included.
    len: u64,
    /// The fewest and the most M-cycles the instruction takes.
    cycles: (u64, u64),
}

/// Read every instruction of the table at `key`, in opcode order.
fn read_table(json: &Value, key: &str) -> Vec<Instr> {
    let table = json[key]
        .as_object()
        .unwrap_or_else(|| panic!("`{key}` is not an object"));
    let table: BTreeMap<u8, Instr> = table
        .iter()
        .map(|(opcode, instr)| {
            let byte = opcode
                .strip_prefix("0x")
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .unwrap_or_else(|| panic!("`{opcode}` in `{key}` is not an opcode"));
            (byte, read_instr(opcode, instr))
        })
        .collect();
    assert_eq!(table.len(), 256, "`{key}` does not define every opcode");
    let mut table: Vec<Instr> = table.into_values().collect();

    if key == "cbprefixed" {
        for instr in &table {
            assert_eq!(instr.len, 2, "`{}` is not two bytes long", instr.asm);
        }
    } else {
        // the JSON gives the prefix its own length, with the byte after it counted by the
        // prefixed instructions
        let prefix = &mut table[0xCB];
        assert_eq!(prefix.name, "PREFIX", "0xCB is not the prefix");
        prefix.len = 2;
    }
    table
}

fn read_instr(opcode: &str, instr: &Value) -> Instr {
    let mnemonic = instr["mnemonic"]
        .as_str()
        .unwrap_or_else(|| panic!("{opcode} has no mnemonic"));
    let len = instr["bytes"]
        .as_u64()
        .unwrap_or_else(|| panic!("{opcode} has no length"));
    assert!((1..=3).contains(&len), "{opcode} has a length of {len}");

    let cycles: Vec<u64> = instr["cycles"]
        .as_array()
        .unwrap_or_else(|| panic!("{opcode} has no cycles"))
        .iter()
        .map(|cycles| {
            let t_states = cycles
                .as_u64()
                .unwrap_or_else(|| panic!("{opcode} has cycles which are not a number"));
            assert_eq!(
                t_states % T_STATES_PER_M_CYCLE,
                0,
                "{opcode} does not take a whole number of M-cycles"
            );
            t_states / T_STATES_PER_M_CYCLE
        })
        .collect();
    let (Some(&min), Some(&max)) = (cycles.iter().min(), cycles.iter().max()) else {
        panic!("{opcode} has no cycles");
    };

    let mut name = mnemonic.to_owned();
    let mut asm = mnemonic.to_owned();
    let mut separator = " ";
    let operands = instr["operands"]
        .as_array()
        .unwrap_or_else(|| panic!("{opcode} has no operands"));
    for operand in operands {
        let operand_name = operand["name"]
            .as_str()
            .unwrap_or_else(|| panic!("{opcode} has an operand with no name"));
        let direct = operand["immediate"]
            .as_bool()
            .unwrap_or_else(|| panic!("{opcode} has an operand which is neither direct nor not"));
        let step = if operand["increment"].as_bool() == Some(true) {
            Some(('I', '+'))
        } else if operand["decrement"].as_bool() == Some(true) {
            Some(('D', '-'))
        } else {
            None
        };

        name.push('_');
        name.push_str(&operand_name.trim_start_matches('$').to_uppercase());
        asm.push_str(separator);
        separator = ", ";
        match (direct, step) {
            (true, None) => asm.push_str(operand_name),
            // the register the offset in the next operand is added to
            (true, Some((_, sign))) => {
                write!(asm, "{operand_name}{sign}").unwrap();
                separator = "";
            }
            (false, None) => {
                name.push_str("_PTR");
                write!(asm, "[{operand_name}]").unwrap();
            }
            (false, Some((suffix, _))) => {
                write!(name, "{suffix}_PTR").unwrap();
                write!(asm, "[{operand_name}{suffix}]").unwrap();
            }
        }
    }

    Instr {
        mnemonic: mnemonic.to_owned(),
        name,
        asm,
        len,
        cycles: (min, max),
    }
}

/// Check every instruction got a name which is an identifier and which no other one shares.
fn check_names(unprefixed: &[Instr], prefixed: &[Instr]) {
    let mut names = BTreeMap::new();
    for instr in unprefixed.iter().chain(prefixed) {
        assert!(
            instr
                .name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
            "`{}` is named `{}`, which is not an identifier",
            instr.asm,
            instr.name
        );
        if let Some(other) = names.insert(&instr.name, &instr.asm) {
            panic!(
                "`{other}` and `{}` are both named `{}`",
                instr.asm, instr.name
            );
        }
    }
}

fn write_constants(out: &mut String, table: &[Instr], ty: &str, value: impl Fn(usize) -> String) {
    for (opcode, instr) in table.iter().enumerate() {
        writeln!(out, "/// `{}`", instr.asm).unwrap();
        writeln!(out, "pub const {}: {ty} = {};", instr.name, value(opcode)).unwrap();
    }
}

fn write_table(
    out: &mut String,
    name: &str,
    ty: &str,
    table: &[Instr],
    value: impl Fn(&Instr) -> String,
) {
    writeln!(out, "const {name}: [{ty}; 256] = [").unwrap();
    for instr in table {
        writeln!(out, "    {},", value(instr)).unwrap();
    }
    writeln!(out, "];").unwrap();
}
