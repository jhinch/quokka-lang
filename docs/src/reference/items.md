# Items

```ebnf
Item        =   SimpleItem
            |   Attribute { Attribute } SimpleItem
            ;
SimpleItem  =   ConstItem
            |   GlobalItem
            |   Struct
            |   Function
            ;
```