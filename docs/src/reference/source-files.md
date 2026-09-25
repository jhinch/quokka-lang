# Source files

A collection of 1 or more source files describe a program, the unit of compilation for Quokka. When compiled, a
program will produce a binary executable (a "ROM"). 

## Source file

```ebnf
SourceFile =   ? EOF ?
           |   Item , { Item } , ? EOF ?
           ;
```

## Entry file

The entry file is a special source file and the only mandatory file required for a Quokka program. It defines the root
module. The root module must contain the entrypoint function of the program. There is exactly one entrypoint function
for a program. Any other number of entrypoint functions will result in a compilation error.

In addition to the entrypoint function, the entry file defines the zero or more program attributes. Program attributes
apply to the entire program rather than a single [item](items.md).

The entry file is typically named `main.q` and placed within the `src/` directory from the project's root directory.

## Entrypoint function

The entrypoint function defines the code which will be executed immediately proceeding the boot ROM and the
[startup code](#startup-code). The entrypoint function must contain zero arguments.

```quokka
fn main() {
    // program goes here
}
```

> [!WARNING]
> Requires `custom-entrypoint` preview feature enabled.

The default entrypoint function is named `main`. This can be customized using the `#[entrypoint]` attribute

```quokka,preview=custom-entrypoint
#[entrypoint]
fn my_entrypoint() {
    // program goes here
}
```

The entrypoint function is resolved using the following steps:

1. Find all functions within the program with an `#[entrypoint]` attribute.
2. A compilation error occurs there is more than one function annotated with the `#[entrypoint]` attribute or there is
   a function annotated with the `#[entrypoint]` attribute outside the root module
3. If no function is annotated with the `#[entrypoint]` attribute, find a function called `main` in the root module.

## Startup code

The startup code are additional instructions added to the resulting ROM which allows for the entrypoint function to
behave like a regular quokka function. The exact instructions are unspecified.
