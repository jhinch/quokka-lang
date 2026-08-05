# Attributes

```ebnf
Attribute       =   "#" , "[" , Identifier , [ "=" , AttributeValue ] , "]"
AttributeValue  =   Identifier
                |   NumberLiteral
                ;
```

```quokka
#[this_attribute]
#[that_attribute]
fn main() {
}
```