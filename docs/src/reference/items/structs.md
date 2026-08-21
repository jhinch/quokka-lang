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

## Padding/Aignment

Structs will automatically align each field to a new byte with other bits as padding.

Additional padding can also be defined using fields with `_` as their name. Padding fields must be an unsigned int.
Multiple padding fields can be defined.

```quokka
// 32-bit struct
struct Padded {
    one: u2, // bit 0-1 used, bit 2-7 padding
    _: u8, // bit 8-15 padding 
    _: u8, // bit 26-23 padding
    two: u8, // bit 24-31 used
}
```