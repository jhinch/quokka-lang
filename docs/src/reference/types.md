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
- User-defined structure types (`struct` types)
- Array types
- Function pointer types

## Primitive types

- Numbers
  - `u1`
  - `u2` (requires `narrow-int` preview feature enabled)
  - `u4` (requires `narrow-int` preview feature enabled)
  - `u8`
  - `u16`
  - `i8`
  - `i16`
- `never`
- `unit`

## Array types

i.e. `[ItemType; Count]`

## User-defined structure types (`struct` types)

See [Structs](items/structs.md)

## Function pointer types

See [Functions](items/functions.md)