import {
    Choice,
    Diagram, Group,
    NonTerminal,
    OneOrMore,
    Optional,
    Sequence,
    Skip,
    Terminal,
    ZeroOrMore
} from "./railroad";

export default class Parser {
    constructor(src) {
        this.src = src;
        this.lexer = new RegExp([
            ['WS', /\s+/],
            ['IDENT', /[a-zA-Z][a-zA-Z0-9]*/],
            ['SPEC_SEQ', /\?[^?]*\?/],
            ['INTEGER', /[0-9]+/],
            ['STRING', /"[^"]+"|'[^']+'/],
            ['EQUAL', /=/],
            ['SEMICOLON', /;/],
            ['PIPE', /\|/],
            ['COMMA', /,/],
            ['MINUS', /-/],
            ['STAR', /\*/],
            ['LEFT_BRACKET', /\[/],
            ['RIGHT_BRACKET', /]/],
            ['LEFT_BRACE', /{/],
            ['RIGHT_BRACE', /}/],
            ['LEFT_PAREN', /\(/],
            ['RIGHT_PAREN', /\)/],
            ['INVALID', /./],
        ].map(([kind, regex]) => `(?<${kind}>${regex.source})`).join('|'), 'y');
        this.peeked = null;
        this.peekedText = null;
        this.current = null;
        this.currentText = null;
    }

    peekToken() {
        let peeked = this.peeked;
        if (peeked) {
            return peeked;
        }
        if (this.current === 'EOF') {
            throw "EOF already reached";
        }
        while (true) {
            let match = this.lexer.exec(this.src);
            let text = '';
            if (match) {
                let [kind, t] = Object.entries(match.groups).find(([_, b]) => b !== undefined);
                if (kind === 'INVALID') {
                    throw `unexpected character ${t}`;
                }
                if (kind === 'WS') {
                    continue;
                }
                peeked = kind;
                text = t;
            } else {
                peeked = 'EOF';
            }
            this.peeked = peeked;
            this.peekedText = text;
            return peeked;
        }
    }

    consumeToken() {
        let kind = this.peekToken();
        this.current = kind;
        this.currentText = this.peekedText;
        this.peeked = null;
        this.peekedText = null;
        return kind;
    }

    expectToken(expected) {
        let actual = this.consumeToken();
        if (actual !== expected) {
            throw `expected ${expected}, found ${actual}`;
        }
    }

    parse() {
        let diagrams = {};
        do {
            let {name, diagram} = this.parseRule();
            diagrams[name] = diagram;
        } while(this.peekToken() !== 'EOF');
        this.expectToken('EOF');
        return diagrams;
    }

    parseRule() {
        this.expectToken('IDENT');
        let name = this.currentText;
        this.expectToken('EQUAL');
        let node = this.parseRuleDefinitions();
        this.expectToken('SEMICOLON');
        return {name, diagram: new Diagram(new Group(node, name))};
    }

    parseRuleDefinitions() {
        let definitions = [this.parseRuleDefinition()];
        while (this.peekToken() === 'PIPE') {
            this.expectToken('PIPE');
            definitions.push(this.parseRuleDefinition());
        }
        if (definitions.length === 1) {
            return definitions[0];
        }
        return new Choice(0, ...definitions);
    }

    parseRuleDefinition() {
        let terms = [this.parseTerm()];
        while (this.peekToken() === 'COMMA') {
            this.expectToken('COMMA');
            let other = this.parseTerm();
            // detect one or more scenarios defined as a non-terminal follow by a repeated non-terminal
            if (other instanceof Choice && other.items.length === 2 && other.items[0] instanceof Skip) {
                let current = other.items[1];
                let previous = terms[terms.length - 1];
                if (current instanceof OneOrMore && previous instanceof NonTerminal && current.item instanceof NonTerminal) {
                    if (current.item.text === previous.text) {
                        terms[terms.length - 1] = new OneOrMore(previous);
                        continue;
                    }
                }
            }
            terms.push(other);
        }
        if (terms.length === 1) {
            return terms[0];
        }
        return new Sequence(...terms);
    }

    parseTerm() {
        let term = this.parseFactor();
        if (this.peekToken() === 'MINUS') {
            this.expectToken('MINUS');
            let exception = this.parseFactor();
            return new NonTerminal(`${term.text} - ${exception.text}`);
        }
        return term;
    }

    parseFactor() {
        if (this.peekToken() === 'INTEGER') {
            this.expectToken('INTEGER');
            let amount = parseInt(this.currentText, 10);
            this.expectToken('STAR');
            let primary = this.parsePrimary();
            return new OneOrMore(primary, `${amount}x`);
        }
        return this.parsePrimary();
    }

    parsePrimary() {
        switch (this.consumeToken()) {
            case 'IDENT': {
                return new NonTerminal(this.currentText);
            }
            case 'STRING': {
                return new Terminal(this.currentText.substring(1, this.currentText.length - 1));
            }
            case 'SPEC_SEQ': {
                return new Terminal(this.currentText.substring(1, this.currentText.length - 1));
            }
            case 'LEFT_BRACKET': {
                let definitions = this.parseRuleDefinitions();
                this.expectToken('RIGHT_BRACKET');
                return new Optional(definitions);
            }
            case 'LEFT_BRACE': {
                let definitions = this.parseRuleDefinitions();
                this.expectToken('RIGHT_BRACE');
                return new ZeroOrMore(definitions);
            }
            case 'LEFT_PAREN': {
                let definitions = this.parseRuleDefinitions();
                this.expectToken('RIGHT_PAREN');
                return definitions;
            }
            default: {
                throw `unexpected token: ${this.current}`;
            }
        }
    }
}