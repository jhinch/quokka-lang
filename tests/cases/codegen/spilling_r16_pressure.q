global A: u16;
global B: u16;
global C: u16;
global D: u16;
global E: u16;
global RESULT: [u16; 5];

fn f() {
    let a = A;
    let b = B;
    let c = C;
    let d = D;
    let e = E;
    RESULT[0] = a;
    RESULT[1] = b;
    RESULT[2] = c;
    RESULT[3] = d;
    RESULT[4] = e;
}

fn main() {
    f();
}
