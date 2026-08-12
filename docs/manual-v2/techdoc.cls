% -----------------------------------------------------------------------------
% LaTeX Class 'techdoc' - Class for technical documentation
% -----------------------------------------------------------------------------
\NeedsTeXFormat{LaTeX2e}
\ProvidesClass{techdoc}[2026/07/23 Class for technical documentation]
% -----------------------------------------------------------------------------

\newcommand{\@ptoption}{1pt}
\newcommand{\@tipo}{oneside}
\ExecuteOptions{oneside,12pt}
\ProcessOptions\relax

% -----------------------------------------------------------------------------
% Sized for A4 standard paper
% -----------------------------------------------------------------------------
\LoadClass[\@tipo,a4paper,1\@ptoption]{book}

% -----------------------------------------------------------------------------
% Support packages
% -----------------------------------------------------------------------------
\RequirePackage{helvet}
\RequirePackage{times}
\RequirePackage{longtable}

% Page geometry
\RequirePackage[total={165mm,262mm},top=15mm, left=24mm,bindingoffset=0mm, includefoot, includehead,headheight=13.6pt]{geometry}

% Other support
\RequirePackage[nottoc]{tocbibind}
\RequirePackage{titlesec}
\RequirePackage{titletoc}
\RequirePackage{lastpage}
\RequirePackage{enumitem}
\RequirePackage[table]{xcolor}
\RequirePackage{array}
\RequirePackage{graphicx}
\RequirePackage{fancyhdr}
\RequirePackage[final]{listings}
\RequirePackage[font={small,it},margin=10pt,justification=centerlast,position=below]{caption}
\RequirePackage[pdftex]{hyperref}

% Paragraph layout
\setlength{\parindent}{0pt}
\setlength{\parskip}{0.65\baselineskip plus 0.1\baselineskip minus 0.05\baselineskip}
\setlist[itemize]{itemsep=0.15\baselineskip,topsep=0.3\baselineskip,parsep=0pt,partopsep=0pt}
\setlist[enumerate]{itemsep=0.15\baselineskip,topsep=0.3\baselineskip,parsep=0pt,partopsep=0pt}
\setlist[description]{style=unboxed,leftmargin=0pt,labelsep=0.5em,itemsep=0.1\baselineskip,topsep=0.2\baselineskip,parsep=0pt,partopsep=0pt}

% ---------------------------------------------------------------------------
% --- Numbering
% ---------------------------------------------------------------------------
\setcounter{secnumdepth}{3}
\setcounter{tocdepth}{2}

\makeatletter
\renewcommand{\thesection}{%
  \ifnum\c@chapter<1 \@arabic\c@section
  \else \thechapter.\@arabic\c@section
  \fi
}
\makeatother

% ---------------------------------------------------------------------------
% --- Listings
% ---------------------------------------------------------------------------
\definecolor{dkgreen}{rgb}{0,0.6,0}
\definecolor{gray}{rgb}{0.5,0.5,0.5}
\definecolor{mauve}{rgb}{0.58,0,0.82}
\definecolor{light-gray}{gray}{0.95}
\definecolor{iptoInk}{HTML}{0B1220}
\definecolor{iptoMuted}{HTML}{334155}
\definecolor{iptoAccent}{HTML}{0EA5A4}

\arrayrulecolor{iptoMuted!20}
\renewcommand{\arraystretch}{1.25}

\lstdefinestyle{rustCode}{%
    language={},
    frame=none,
    aboveskip=3mm,
    belowskip=3mm,
    basicstyle=\footnotesize\ttfamily,
    keywordstyle=\ttfamily\bf,
    keepspaces=true,
    showspaces=false,
    showstringspaces=false,
    showtabs=false,
    stringstyle=\itshape,
    flexiblecolumns=true,
    commentstyle=\itshape,
    captionpos=below,
    abovecaptionskip=10pt,
    belowcaptionskip=0pt,
    backgroundcolor=\color{light-gray},
    keywordstyle=\color{blue},
    commentstyle=\color{dkgreen},
    stringstyle=\color{mauve},
    escapeinside={(*}{*)},
    numbers=left,
    numberstyle={\tiny},
    columns=flexible,
}

% ---------------------------------------------------------------------------
% --- Empty pages
% ---------------------------------------------------------------------------
\let\origdoublepage\cleardoublepage
\newcommand{\clearemptydoublepage}{%
    \clearpage
    {\pagestyle{empty}\origdoublepage}%
}
\let\cleardoublepage\clearemptydoublepage

\newenvironment{compressed_itemize}{
    \vspace{-0.2cm}\begin{itemize}
    \setlength{\itemsep}{0pt}
    \setlength{\parskip}{0pt}
    \setlength{\parsep}{0pt}
}
{
    \end{itemize}
}

\newenvironment{compressed_enumerate}{
    \vspace{-0.2cm}\begin{enumerate}
    \setlength{\itemsep}{0pt}
    \setlength{\parskip}{0pt}
    \setlength{\parsep}{0pt}
}
{
    \end{enumerate}
}

% ---------------------------------------------------------------------------
% --- FRONT PAGE
% ---------------------------------------------------------------------------
\newcommand{\mychaptertitlefont}{\fontseries{bx}\fontsize{20}{24}\selectfont}
\newcommand{\mytitlefont}{\fontseries{b}\sc\fontsize{15}{24}\selectfont}
\newcommand{\mysubtitlefont}{\fontseries{m}\fontsize{13}{16}\selectfont}
\newcommand{\authorfont}{\fontseries{m}\fontsize{14}{14}\selectfont}
\newcommand{\largefont}{\fontseries{m}\fontsize{18}{14}\selectfont}
\newcommand{\bigfont}{\fontseries{m}\fontsize{23}{14}\selectfont}
\newcommand{\biggerfont}{\fontseries{m}\fontsize{26}{22}\selectfont}

\makeatletter
\def\title#1{\gdef\@thetitle{#1}}
\def\subtitle#1{\gdef\@thesubtitle{#1}}
\def\editors#1{\gdef\@theeditors{#1}}

\def\@theauthors{}
\def\@thetitle{}
\def\@thesubtitle{}

\newcommand{\HRule}{\rule{\linewidth}{0pt}}
\newcommand{\filepath}[1]{%
    \begingroup
    \def\UrlBreaks{\do\/\do\.\do\_\do\-}%
    \nolinkurl{#1}%
    \endgroup
}
\newcommand{\shellcmd}[1]{%
    \begingroup
    \def\UrlBreaks{\do\/\do\.\do\_\do\-\do\ }%
    \nolinkurl{#1}%
    \endgroup
}
\newcommand{\literalbreak}[1]{%
    \begingroup
    \def\UrlBreaks{\do\.\do\_\do\-}%
    \nolinkurl{#1}%
    \endgroup
}

\renewcommand{\maketitle}{%
    \thispagestyle{empty}
    \setlength{\parindent}{0mm}

    \begin{center}
    \huge\textsc{\@thetitle{}}
    \end{center}

    \vspace*{5cm}

    \begin{center}
    \huge\textsc{\@thesubtitle{}}
    \end{center}

    \vspace*{12cm}

    \begin{center}
    \today
    \end{center}
}

\makeatother

% ---------------------------------------------------------------------------
%    Headers
% ---------------------------------------------------------------------------
\pagestyle{fancy}
\renewcommand{\headrulewidth}{0.4pt}
\renewcommand{\footrulewidth}{0.4pt}

% ---------------------------------------------------------------------------
% --- MISC COMMANDS
% ---------------------------------------------------------------------------
\titleformat{name=\chapter,numberless=false}[display]
{\large}
{\vspace{-12mm}}
{1ex}{\mychaptertitlefont \thechapter \hspace{4mm}}

\titleformat{name=\chapter,numberless=true}[display]
{\large}
{\vspace{-12mm}}
{1ex}{\mychaptertitlefont}

\titlespacing{\chapter}{0mm}{*0}{*1}

\titlecontents{chapter}[0pt] {\addvspace{2mm} \rmfamily
\Large}{\thecontentslabel\quad}{}{\titlerule*[5pt]{.}\contentspage}

\titlecontents{section}[4mm] {\rmfamily
\large}{\thecontentslabel\quad}{}{\titlerule*[5pt]{.}\contentspage}

\titlecontents{subsection}[9mm] {\rmfamily}
{\thecontentslabel\quad}{}{\titlerule*[5pt]{.}\contentspage}

\titlecontents{figure}[0pt] {\addvspace{4mm} \rmfamily}{Figure
\thecontentslabel :\enspace}{}{\titlerule*[5pt]{.}\contentspage}

\titlecontents{table}[0pt] {\addvspace{4mm} \rmfamily}{Table
\thecontentslabel :\enspace}{}{\titlerule*[5pt]{.}\contentspage}
