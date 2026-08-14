# Let statements

```ebnf
LetStatement    =   "let" , Identifier , [ ":" , TypeUse ] , "=" , Expression , ";"
                ;
```

## Restrictions

Quokka has a size restriction on what types can be stored in local variables due to the target hardware's limitations.
When declared within functions, the type of a let declaration _must_ be less than or equal to 16 bits.
This limitation does apply to const declarations which are evaluated at compile time and as a result can be larger.