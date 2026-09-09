# Const items

```ebnf
ConstItem   =   "const" , Identifier , ":" , TypeUse , "=" , Expression , ";"
            ;
```

```quokka
const data: u8 = 1;
```

A constant represents a value known to the compiler which doesn't change. Constant declarations must include an
expression which, when evaluated, will result in a value. The expression is evaluated at compile time and cannot
rely on runtime items such as [global variables](global-items.md) or [functions](functions.md).

## Inlining

The compiler may inline constant values into their uses. Constants which are still referenced at runtime will be
be stored in read only memory (ROM) and loaded when used.

## Using specific regions of memory

By default, when a constant is retained at runtime, it will be placed within the `ROM0` region at compiler chosen
address. However, the `region` and `address` attributes can be used place the constant in a different region of
read only memory. If the `address` is specified, the constant will be written to read only memory at that address
regardless of whether it is used or not.