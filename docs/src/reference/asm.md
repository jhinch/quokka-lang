# Assembly instructions

> [!WARNING]
> Requires `inline-asm` preview feature enabled.

```ebnf
ADCInstruction      =   "ADC" Identifier IdentDerefOrNum                        ;
ADDInstruction      =   "ADD" (Identifier | "SP") ( IdentDerefOrNum | "SP" )    ;
ANDInstruction      =   "AND" Identifier IdentDerefOrNum                        ;
BITInstruction      =   "BIT" NumberLiteral IdentOrDeref                        ;
CALLInstruction     =   "CALL" [ JumpCondition ] Path                           ;
CCFInstruction      =   "CCF"                                                   ;
CPInstruction       =   "CP" Identifier IdentDerefOrNum                         ;

IdentOrDeref        =   Identifier
                    |   "[" Identifier "]"
                    ;
IdentDerefOrNum     =   IdentOrDeref
                    |   NumberLiteral
                    ;
JumpCondition       =   "Z"
                    |   "NZ"
                    |   "C"
                    |   "NC"
                    ;
```