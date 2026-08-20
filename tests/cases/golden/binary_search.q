global SORTED: [u8; 8];
global TARGET: u8;

fn binary_search() -> u16 {
    let low = 0;
    let high = 7;
    loop {
        if high < low then {
            return 255;
        }
        let mid = (low + high) >>;
        let value = SORTED[mid];
        if value == TARGET then {
            return mid;
        }
        if value < TARGET then {
            low = mid + 1;
        } else {
            high = mid - 1;
        }
    }
}

global FOUND_INDEX: u16;

fn main() {
    SORTED[0] = 1;
    SORTED[1] = 3;
    SORTED[2] = 4;
    SORTED[3] = 7;
    SORTED[4] = 9;
    SORTED[5] = 12;
    SORTED[6] = 15;
    SORTED[7] = 20;
    TARGET = 12;
    FOUND_INDEX = binary_search();
}
