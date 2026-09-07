# Const items

```ebnf
ConstItem   =   "const" , Identifier , ":" , TypeUse , "=" , Expression , ";"
            ;
```

```quokka
const data: u8 = 1;
```

A constant represents a value known to the compiler which doesn't change. For values 16 bits or smaller, the value
will be inlined into anywhere it is used. For larger values, the data is placed into read only memory ROM and
referenced at runtime.