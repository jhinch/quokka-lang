# ASM block expressions

> [!WARNING]
> Requires `inline-asm` preview feature enabled.

```ebnf
AsmBlockExpression  =   "asm" "{" AsmInstruction { AsmInstruction } "}"   ;
```