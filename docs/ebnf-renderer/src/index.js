import Parser from "./parser";

document.addEventListener('DOMContentLoaded', () => {
    document.querySelectorAll("code.language-ebnf").forEach(codeElem => {
        const preElem = codeElem.closest("pre");
        const outputElem = document.createElement("div");
        try {
            const parser = new Parser(codeElem.innerText);
            const diagrams = parser.parse();
            Object.values(diagrams).forEach(diagram => {
                const sectionElem = document.createElement("section");
                sectionElem.append(diagram.toSVG());
                outputElem.append(sectionElem);
            })
        } catch (e) {
            outputElem.innerText = 'Error: ' + e;
            outputElem.style.color = 'red';
        }
        preElem.after(outputElem);
    })
});
