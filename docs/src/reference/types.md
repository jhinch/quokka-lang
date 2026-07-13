# Types

```ebnf
TypeUse         =   SimpleTypeUse
                |   ArrayTypeUse
                ;
SimpleTypeUse   =   Identifier                          ;
ArrayTypeUse    =   "[" TypeUse ";" NumberLiteral "]"   ;
```