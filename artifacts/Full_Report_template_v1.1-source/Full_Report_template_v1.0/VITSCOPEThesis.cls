%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
% This class provides the Vellore Institute of Technology - 
% School of Computer Science and Engineering Students 
% Thesis format. 
% This class was prepared by Dr. SUDHAKAR. P based on the 
% guidance from Dr. Kalyanaraman P 
% %%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%

\NeedsTeXFormat{LaTeX2e}
\ProvidesClass{VITSCOPEThesis}[2026/02/13 VIT-Vellore SCOPE Thesis or Project report format]
\LoadClass[12pt]{report} % onehalfspacing

% Document Layout Settings
\RequirePackage[a4paper,left=1.5in,right=1in,top=1in,bottom=1in,nohead]{geometry}
\RequirePackage{setspace}
\setstretch{1.5}
% Declared for necessary formatting
\RequirePackage{mathptmx} % For times font
\RequirePackage{graphicx}

%Table of contents, List of figures, Tables 
\renewcommand*\contentsname{\MakeUppercase{Table of Contents}}
\renewcommand{\listtablename}{List of Tables}
\renewcommand{\listfigurename}{List of Figures}

% Chapter, section and subsection head formatting
\RequirePackage{titlesec}
% Chapter 
\titleformat{\chapter}[block]{\normalfont\Large\bfseries}{CHAPTER \thechapter}{0.3em}{\linebreak  \centering\MakeUppercase}
\titlespacing*{\chapter}{0pt}{-1em}{0pt}
% Section 
\titleformat{\section}[block]{\normalfont\bfseries}{\thesection}{0.3em}{\MakeUppercase}
% Subsection
\titleformat{\subsection}[block]{\normalfont\bfseries}{\thesubsection}{0.3em}{}

% other packages
\RequirePackage{subcaption}
\RequirePackage{natbib}
\RequirePackage{array}
\RequirePackage[sort]{acro}
\acsetup{
	list/template=tabular
}
\RequirePackage{longtable}
\RequirePackage{algorithm}
\RequirePackage{algpseudocode}
\RequirePackage{listings}
\RequirePackage{titlecaps}
\RequirePackage{pdfpages}
% other packages
\RequirePackage{listings}
\RequirePackage{ragged2e}
\RequirePackage{ifthen}
\RequirePackage{rotating}
\lstset{captionpos=b,
	frame=tb,
	basicstyle=\scriptsize\ttfamily,
	showstringspaces=false,
	keepspaces=true}
% Bib formatting 
\bibliographystyle{apalike}

% Source code formatting 

