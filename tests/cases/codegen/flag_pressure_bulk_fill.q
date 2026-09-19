global FILLED: [u8; 5];

global X: u8;
global Y: u8;
global Z: u8;

global R_EQ: u1;
global R_NEQ: u1;

fn main() {
    let eq = X == Y;
    let neq = X == Z;

    FILLED = [7; 5];

    R_EQ = eq;
    R_NEQ = neq;
}
