global DATA: [u8; 8];

fn bubble_sort() -> u8 {
    let swaps = 0;
    let i = 0;
    loop {
        if i >= 8 then {
            break;
        }
        let limit = 8 - i - 1;
        let j = 0;
        loop {
            if j >= limit then {
                break;
            }
            if DATA[j + 1] < DATA[j] then {
                let tmp = DATA[j];
                DATA[j] = DATA[j + 1];
                DATA[j + 1] = tmp;
                swaps += 1;
            }
            j += 1;
        }
        i += 1;
    }
    return swaps;
}

global SWAP_COUNT: u8;

fn main() {
    DATA[0] = 5;
    DATA[1] = 3;
    DATA[2] = 8;
    DATA[3] = 1;
    DATA[4] = 9;
    DATA[5] = 2;
    DATA[6] = 7;
    DATA[7] = 4;
    SWAP_COUNT = bubble_sort();
}
