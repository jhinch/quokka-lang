# Tokens

- [Keywords](#keywords)
- [Identifiers](#identifiers)
- [Literals](#literals)
- [Separators](#separators)
- [Operators](#operators)

## Keywords

```ebnf
Keyword             =   StandardKeyword
                    |   AsmKeyword
                    ;
Reserved            =   Keyword
                    |   UnusedReserved
                    ;
StandardKeyword     =   "asm" | "const" | "else" | "fn"
                    |   "global" | "if" | "let" | "loop"
                    |   "struct" | "then" | "yield" | "break"
                    |   "return" | "continue"
                    ;
AsmKeyword          =   "ADC" | "ADD" | "AND" | "BIT" | "CALL"
                    |   "CCF" | "CP" | "CPL" | "DAA" | "DEC"
                    |   "DI" | "EI" | "HALT" | "INC" | "JP"
                    |   "JR" | "LD" | "LDH" | "NOP" | "OR"
                    |   "POP" | "PUSH" | "RES" | "RET" | "RETI"
                    |   "RL" | "RLA" | "RLC" | "RLCA" | "RR"
                    |   "RRA" | "RRC" | "RRCA" | "RST" | "SBC"
                    |   "SCF" | "SET" | "SLA" | "SRA" | "SRL"
                    |   "STOP" | "SUB" | "SWAP" | "XOR" | "A"
                    |   "HL" | "SP" | "Z" | "NZ" | "C"
                    |   "NC"
                    ;
UnusedReserved      =    "_" | "as" | "while" | "do" | "match"
                    |   "case" | "enum" | "false" | "true" | "for"
                    |   "switch" | "in" | "void" | "to" | "mod"
                    |   "root"
                    ;
```

## Identifiers

```ebnf
Identifier              = IdentifierOrReserved - Reserved
                        ;
IdentifierOrReserved    = ( XIDStart | "_" ) , { XIDContinue }
                        ;
XIDStart                = ? as define in https://www.unicode.org/reports/tr31/tr31-41.html ?
                        ;
XIDContinue             = ? as define in https://www.unicode.org/reports/tr31/tr31-41.html ?
                        ;
```

Identifiers follow the [Unicode Identifier Standard](https://www.unicode.org/reports/tr31/tr31-41.html) with the one
addition that the first character can be an underscore (`_`).

Identifiers may not be a [reserved word](#keywords) including keywords and unused reserved words.

## Literals

### Number literals

```ebnf
NumberLiteral   =   DecimalNumber
                |   BinaryNumber
                |   OctalNumber
                |   HexNumber
                ;

DecimalNumber   =   DecimalDigit , { DecimalDigit | "_" }
                ;
BinaryNumber    =   "0b" , { BinaryDigit | "_" } , BinaryDigit , { BinaryDigit | "_" }
                ;
OctalNumber     =   "0o" , { OctalDigit | "_" } , OctalDigit , { OctalDigit | "_" }
                ;
HexNumber       =   "0x" , { HexDigit | "_" } , HexDigit , { HexDigit | "_" }             
                ;
BinaryDigit     =   "0" | "1"
                ;
OctalDigit      =   "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7"
                ;
DecimalDigit    =   "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
                ;
HexDigit        =   "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" 
                |   "a" | "b" | "c" | "d" | "e" | "f" 
                |   "A" | "B" | "C" | "D" | "E" | "F"
                ;
```

## Separators

- `{` and `}`
- `[` and `]`
- `(` and `)`
- `<` and `>`
- `,`
- `:`
- `;`
- `->`

## Operators

- `=`
- `+` and `-`
- `+=` and `-=`
- `||` and `&&`
- `|`, `&` and `^`
- `|=`, `&=` and `^=`
- `==`, `!=`, `<`, `<=`, `>=` and `>`
- `!`
- `<<`, `>>`, `<%` and `>%`
- `.`
