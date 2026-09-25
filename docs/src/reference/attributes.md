# Attributes

```ebnf
Attribute       =   "#" , "[" , Identifier , [ "=" , AttributeValue ] , "]"
AttributeValue  =   Identifier
                |   NumberLiteral
                ;
```

```quokka
#[region = rom0]
#[address = 0x0150]
fn checkpoint() {
}
```