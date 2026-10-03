; Scenarios for tests/hooks.rs. Each one lives at a fixed address so the tests can name
; addresses directly; the bytes each instruction assembles to are listed beside it.
;
; Rebuild hooks.gb with ./build.sh after changing this file.

SECTION "entry", ROM0[$0100]
    nop                 ; $0100: 00
    jp Events           ; $0101: C3 50 01

SECTION "header", ROM0[$0104]
    ds $4C, $00         ; filled in by rgbfix

; One of each kind of bus access: operand fetches, a data write, a data read, and a
; read-modify-write.
SECTION "events", ROM0[$0150]
Events:
    ld a, $42           ; $0150: 3E 42
    ld [$C000], a       ; $0152: EA 00 C0
    ld hl, $C000        ; $0155: 21 00 C0
    ld b, [hl]          ; $0158: 46
    inc [hl]            ; $0159: 34
.done:
    jr .done            ; $015A: 18 FE

; Interrupt master enable and HALT with nothing pending.
SECTION "flags", ROM0[$0200]
Flags:
    ei                  ; $0200: FB
    nop                 ; $0201: 00
    di                  ; $0202: F3
    halt                ; $0203: 76

; HALT with interrupts disabled and one pending, which triggers the HALT bug: the byte
; after HALT is executed twice, so B ends up as 2.
SECTION "halt bug", ROM0[$0210]
HaltBug:
    di                  ; $0210: F3
    ld b, $00           ; $0211: 06 00
    ld a, $01           ; $0213: 3E 01
    ldh [$FFFF], a      ; $0215: E0 FF
    ldh [$FF0F], a      ; $0217: E0 0F
    halt                ; $0219: 76
    inc b               ; $021A: 04
.done:
    jr .done            ; $021B: 18 FE
