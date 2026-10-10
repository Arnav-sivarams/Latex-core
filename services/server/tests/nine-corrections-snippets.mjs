import { buildTable, buildLongTable, buildFigure, buildWrapFigure, buildEquation, buildPlot, buildAlgorithm, buildCodeListing, buildTheorem, buildBibtexEntry, buildPublicationBibitems } from '../frontend/writer-productivity.mjs';
import { MATH_CATALOG } from '../frontend/math-catalog.mjs';
import { mathInsertionSource } from '../frontend/writer-capabilities.mjs';
const snippets = [
  '\\section{Supported Insert categories}\\label{sec:test}',
  buildTable({rows:2,columns:2,booktabs:true,caption:'Table'}),
  buildLongTable({rows:2,columns:2,header:true}),
  '\\begin{itemize}\\item Supported\\end{itemize}',
  '\\begin{enumerate}\\item Supported\\end{enumerate}',
  '\\(x+y\\)', buildEquation({type:'display'}), buildEquation({type:'aligned'}),
  buildEquation({type:'matrix'}), buildEquation({type:'cases'}),
  buildFigure({asset:'logo.png',caption:'Figure'}), buildWrapFigure({asset:'logo.png'}),
  buildPlot({asset:'data.csv'}), buildAlgorithm(),
  buildAlgorithm({family:'algorithmic'}), buildCodeListing({code:'print(1)'}),
  buildTheorem({environment:'theorem'}), buildTheorem({environment:'proof'}),
  '% Commented line\nUncommented text.', '\\ref{sec:test} \\cite{sample}',
  '\\begin{thebibliography}{9}\n' + buildPublicationBibitems([{citation_key:'publication',status:'published',authors:'A Writer',title:'Research',venue:'Journal',year:2026}]) + '\n\\end{thebibliography}',
  ...MATH_CATALOG.map(entry => mathInsertionSource(entry.latex)+'\n'),
];
const source = '\\documentclass{article}\n\\usepackage{amsmath,amssymb,graphicx,booktabs,longtable,wrapfig,pgfplots,algorithm,algpseudocode,listings,amsthm}\n\\pgfplotsset{compat=1.18}\n\\newtheorem{theorem}{Theorem}\n\\begin{document}\n' + snippets.filter(s=>!s.includes('\\STATE')).join('\n') + '\n\\bibliographystyle{plain}\n\\bibliography{references}\n\\end{document}\n';
const legacy = '\\documentclass{article}\n\\usepackage{algorithm,algorithmic}\n\\begin{document}\n' + buildAlgorithm({family:'algorithmic'}) + '\n\\end{document}\n';
console.log(JSON.stringify({source,legacy,bib:buildBibtexEntry({key:'sample',title:'Sample',author:'A Writer',year:'2026'}),csv:'x,y\n1,2\n2,3\n',categories:19,symbols:MATH_CATALOG.length}));
