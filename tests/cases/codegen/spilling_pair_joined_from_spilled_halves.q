global A: u16;
global B: u16;
global C: u16;
fn f() -> u16 {
    let a = A;
    let b = B;
    let c = C;
    let r0 = a;
    let r1 = b;
    let r2 = c;
    let v0 = b + 1;
    r1 = b ^ 1;
    loop {
        let v1 = v0 + c;
        let v2 = !c;
        break;
    }
    let v3 = b + 65535;
    let v4 = 7 - v3;
    let v5 = 10 ^ v3;
    return (r0 ^ r1) + r2;
}
fn main() {
    f();
}
