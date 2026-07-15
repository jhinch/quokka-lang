# Structs

```ebnf
Struct              =   "struct" , Identifier , "{" , [ StructFields ] , "}"
                    ;
StructFields        =   StructField , { "," , StructField } , [ "," ]
                    ;
StructField         =   SimpleStructField
                    |   Attribute , { Attribute } , SimpleStructField
                    ;
SimpleStructField   =   Identifier , ":" , TypeUse
                    ;
```

```quokka
struct Name {
    field: u8,
}
```