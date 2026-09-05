global A: u8;
global B: u8;
global C: u8;
global D: u8;
global RESULT: [u1; 3];

fn f() {
    let p = A == B;
    let q = B == C;
    let r = C == D;
    RESULT[0] = p;
    RESULT[1] = q;
    RESULT[2] = r;
}

fn main() {}
