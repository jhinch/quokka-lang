# Expressions

An expression is an element of the program which can be evaluated to produce a value.

An expression can have effects ("side effects") during evaluation.

The meaning of each kind of expression dictates:

- whether the expression contains sub-expressions (operands)
- under what circumstances operands are evaluated (including re-evaluation)
- how the values produced by operands combine to obtain the expressions value

## Expression precedence

The order in which expressions are evaluated is governed by a set of conventions.

- expression types can be grouped and ranked into levels
- this rank is called the expression precedence
- levels are ranked from _strongest_ (highest) to _weakest_ (lowest) precedence, where stronger precedence expressions
  are evaluated before weaker precedence
- [binary operator expressions](expressions/operators.md) within the same level are further ordered by the associativity,
  either from _left to right_, from _right to left_, or neither in which case they _require parentheses_
- expression precedence can be overridden using parentheses forming a [grouped expression](expressions/grouped.md)

| Expression type                                                | Associativity        |
|----------------------------------------------------------------|----------------------|
| [Grouped](expressions/grouped.md)                              | N/A                  |
| Paths                                                          | N/A                  |
| Method calls                                                   | N/A                  |
| Field access                                                   | left to right        |
| Unary `-`, `!`                                                 | N/A                  |
| Binary `-`, `+`                                                | left to right        |
| `<<`, `>>`, `<%`, `>%`                                         | left to right        |
| `&`                                                            | left to right        |
| `^`                                                            | left to right        |
| `\|`                                                           | left to right        |
| `==`, `!=`, `<`, `>`, `<=`, `>=`                               | requires parentheses |
| `&&`                                                           | left to right        |
| `\|\|`                                                         | left to right        |
| `=`, `+=`, `-=`, `&=`, `\|=`, `^=`, `<<=`, `>>=`, `<%=`, `>%=` | right to left        |
