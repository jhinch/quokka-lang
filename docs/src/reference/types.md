# Types

```ebnf
TypeUse         =   SimpleTypeUse
                |   ArrayTypeUse
                ;
SimpleTypeUse   =   Identifier
                ;
ArrayTypeUse    =   "[" , TypeUse , ";" , NumberLiteral , "]"
                ;
```

Types define the shape of values within a program.

Every expression, local and global variable, constant has an associated type.

There are four main categories of types:

- Primitive types
- User-defined types (structs)
- Array types

## Primitive types

- Numbers
  - `u1`
  - `u2`
  - `u3`
  - `u4`
  - `u8`
  - `u16`
  - `i8`
- `never`
- `unit`

## Array types

## User-defined types