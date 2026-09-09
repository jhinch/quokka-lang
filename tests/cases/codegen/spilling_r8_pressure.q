global A: u8;
global B: u8;
global C: u8;
global D: u8;
global E: u8;
global F: u8;
global G: u8;
global H: u8;
global RESULT: [u8; 8];

fn f() {
    let a = A;
    let b = B;
    let c = C;
    let d = D;
    let e = E;
    let g = F;
    let h = G;
    let i = H;
    RESULT[0] = a;
    RESULT[1] = b;
    RESULT[2] = c;
    RESULT[3] = d;
    RESULT[4] = e;
    RESULT[5] = g;
    RESULT[6] = h;
    RESULT[7] = i;
}

fn main() {
    f();
}
