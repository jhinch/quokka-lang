; Scenarios for tests/uninit.rs: a short sequence per way of reading memory which has, or has
; not, been written. A test sets PC to the start of a scenario and runs until `Done`; the
; address and bytes of each instruction are listed beside it.
;
; Rebuild uninit.gb with ./build.sh after changing this file.

SECTION "entry", ROM0[$0100]
    nop                 ; $0100: 00
Done:
    jr Done             ; $0101: 18 FE

SECTION "header", ROM0[$0104]
    ds $4C, $00         ; filled in by rgbfix

SECTION "read", ROM0[$0200]
    ld a, [$C000]       ; $0200: FA 00 C0
    jp Done             ; $0203: C3 01 01

SECTION "write then read", ROM0[$0210]
    ld a, $42           ; $0210: 3E 42
    ld [$C000], a       ; $0212: EA 00 C0
    ld a, [$C000]       ; $0215: FA 00 C0
    jp Done             ; $0218: C3 01 01

; Work RAM written through its echo, then a byte of the echo which was not.
SECTION "echo", ROM0[$0220]
    ld a, $42           ; $0220: 3E 42
    ld [$E000], a       ; $0222: EA 00 E0
    ld a, [$C000]       ; $0225: FA 00 C0
    ld a, [$E001]       ; $0228: FA 01 E0
    jp Done             ; $022B: C3 01 01

SECTION "high ram", ROM0[$0230]
    ldh a, [$FF80]      ; $0230: F0 80
    ldh [$FF81], a      ; $0232: E0 81
    ldh a, [$FF81]      ; $0234: F0 81
    jp Done             ; $0236: C3 01 01

SECTION "oam", ROM0[$0240]
    ld [$FE01], a       ; $0240: EA 01 FE
    ld a, [$FE01]       ; $0243: FA 01 FE
    ld a, [$FE00]       ; $0246: FA 00 FE
    jp Done             ; $0249: C3 01 01

; ROM, video RAM, cartridge RAM and the I/O registers.
SECTION "always defined", ROM0[$0250]
    ld a, [$0000]       ; $0250: FA 00 00
    ld a, [$8000]       ; $0253: FA 00 80
    ld a, [$A000]       ; $0256: FA 00 A0
    ldh a, [$FF44]      ; $0259: F0 44
    ldh a, [$FFFF]      ; $025B: F0 FF
    jp Done             ; $025D: C3 01 01

; A pushed value read in place, then read again after it has been popped.
SECTION "pop", ROM0[$0260]
    ld sp, $D000        ; $0260: 31 00 D0
    push bc             ; $0263: C5
    ld a, [$CFFE]       ; $0264: FA FE CF
    pop bc              ; $0267: C1
    ld a, [$CFFF]       ; $0268: FA FF CF
    jp Done             ; $026B: C3 01 01

; A return address read after the return.
SECTION "ret", ROM0[$0270]
    ld sp, $D000        ; $0270: 31 00 D0
    call Return         ; $0273: CD 7C 02
    ld a, [$CFFE]       ; $0276: FA FE CF
    jp Done             ; $0279: C3 01 01
Return:
    ret                 ; $027C: C9

; A two byte frame, written and read while it is allocated, then read after it is freed.
SECTION "add sp", ROM0[$0280]
    ld sp, $D000        ; $0280: 31 00 D0
    add sp, -2          ; $0283: E8 FE
    ld hl, sp + 0       ; $0285: F8 00
    ld [hl], a          ; $0287: 77
    ld a, [hl]          ; $0288: 7E
    add sp, 2           ; $0289: E8 02
    ld a, [hl]          ; $028B: 7E
    jp Done             ; $028C: C3 01 01

; One byte released by stepping SP over it.
SECTION "inc sp", ROM0[$0290]
    ld sp, $D000        ; $0290: 31 00 D0
    push bc             ; $0293: C5
    inc sp              ; $0294: 33
    ld a, [$CFFF]       ; $0295: FA FF CF
    ld a, [$CFFE]       ; $0298: FA FE CF
    jp Done             ; $029B: C3 01 01

; Loading SP switches stacks, leaving the old one as it was.
SECTION "switch stack", ROM0[$02A0]
    ld sp, $D000        ; $02A0: 31 00 D0
    push bc             ; $02A3: C5
    ld sp, $D100        ; $02A4: 31 00 D1
    ld a, [$CFFE]       ; $02A7: FA FE CF
    ld hl, $D200        ; $02AA: 21 00 D2
    ld sp, hl           ; $02AD: F9
    ld a, [$CFFF]       ; $02AE: FA FF CF
    jp Done             ; $02B1: C3 01 01

; The store a program makes to a narrow value at $C010: read the byte, replace the low bit
; and write it back.
SECTION "narrow store", ROM0[$02C0]
    ld a, [$C010]       ; $02C0: FA 10 C0
    and $FE             ; $02C3: E6 FE
    or $01              ; $02C5: F6 01
    ld [$C010], a       ; $02C7: EA 10 C0
    ld a, [$C010]       ; $02CA: FA 10 C0
    jp Done             ; $02CD: C3 01 01

SECTION "narrow read twice", ROM0[$02D0]
    ld a, [$C010]       ; $02D0: FA 10 C0
    ld a, [$C010]       ; $02D3: FA 10 C0
    jp Done             ; $02D6: C3 01 01

SECTION "narrow read once", ROM0[$02E0]
    ld a, [$C010]       ; $02E0: FA 10 C0
    jp Done             ; $02E3: C3 01 01

; An OAM DMA transfer from $C100, of which only $C101 has been written. The transfer is
; started from High RAM, the only memory the CPU can reach while it runs.
SECTION "dma", ROM0[$0300]
Dma:
    ld sp, $D000        ; $0300: 31 00 D0
    ld a, $42           ; $0303: 3E 42
    ld [$C101], a       ; $0305: EA 01 C1
    ld hl, Transfer     ; $0308: 21 21 03
    ld c, $80           ; $030B: 0E 80
    ld b, TransferEnd - Transfer ; $030D: 06 0A
.copy:
    ld a, [hli]         ; $030F: 2A
    ldh [c], a          ; $0310: E2
    inc c               ; $0311: 0C
    dec b               ; $0312: 05
    jr nz, .copy        ; $0313: 20 FA
    call $FF80          ; $0315: CD 80 FF
    ld a, [$FE01]       ; $0318: FA 01 FE
    ld a, [$FE00]       ; $031B: FA 00 FE
    jp Done             ; $031E: C3 01 01

; Runs from $FF80.
Transfer:
    ld a, $C1           ; $0321: 3E C1
    ldh [$FF46], a      ; $0323: E0 46
    ld a, 40            ; $0325: 3E 28
.wait:
    dec a               ; $0327: 3D
    jr nz, .wait        ; $0328: 20 FD
    ret                 ; $032A: C9
TransferEnd:
