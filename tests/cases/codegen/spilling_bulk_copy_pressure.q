const SRC: [u8; 5] = [1, 2, 3, 4, 5];

global DST: [u8; 5];

global A: u8;
global B: u8;
global C: u16;
global D: u16;

global R_A: u8;
global R_B: u8;
global R_C: u16;
global R_D: u16;

fn main() {
    let a = A;
    let b = B;
    let c = C;
    let d = D;

    DST = SRC;

    R_A = a;
    R_B = b;
    R_C = c;
    R_D = d;
}
