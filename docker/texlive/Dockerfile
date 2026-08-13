# Human-readable source tag: debian:13-slim
FROM debian:13-slim@sha256:3a39a0592364683e6bab97937b72cad5a8fa6dcbbee90edb3bb48c7f8e94f258 AS installer
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl xorriso perl && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY docker/texlive/texlive2026-20260301.iso.sha512 ./
RUN curl -fL --retry 5 -o texlive2026-20260301.iso https://mirrors.ctan.org/systems/texlive/Images/texlive2026-20260301.iso \
 && sha512sum -c texlive2026-20260301.iso.sha512 \
 && mkdir iso && xorriso -osirrox on -indev texlive2026-20260301.iso -extract / iso
RUN apt-get update && apt-get install -y --no-install-recommends fontconfig && rm -rf /var/lib/apt/lists/*
COPY docker/texlive/texlive.profile ./texlive.profile
RUN ./iso/install-tl -no-continue -profile ./texlive.profile \
 && /opt/texlive/2026/bin/x86_64-linux/xetex --version >/dev/null \
 && /opt/texlive/2026/bin/x86_64-linux/mktexlsr \
 && /opt/texlive/2026/bin/x86_64-linux/fmtutil-sys --all \
 && /opt/texlive/2026/bin/x86_64-linux/updmap-sys

FROM debian:13-slim@sha256:3a39a0592364683e6bab97937b72cad5a8fa6dcbbee90edb3bb48c7f8e94f258
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates perl fontconfig ghostscript python3 python3-pygments \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --gid 10001 latex-core && useradd --uid 10001 --gid 10001 --no-create-home --home-dir /tmp/home --shell /usr/sbin/nologin latex-core
COPY --from=installer /opt/texlive /opt/texlive
COPY target/release/tex-index /usr/local/libexec/latex-core/tex-index
COPY docker/texlive/compile-entrypoint.sh /usr/local/bin/compile-entrypoint
COPY docker/texlive/safe.latexmkrc docker/texlive/restricted.latexmkrc /opt/latex-core/
ENV PATH=/opt/texlive/2026/bin/x86_64-linux:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
RUN ln -s /usr/local/libexec/latex-core/tex-index /usr/local/bin/tex-index \
 && cp --remove-destination /opt/texlive/2026/texmf-dist/scripts/texlive/tlmgr.pl /opt/texlive/2026/bin/x86_64-linux/tlmgr \
 && chmod 0555 /opt/texlive/2026/bin/x86_64-linux/tlmgr /usr/local/bin/compile-entrypoint /usr/local/libexec/latex-core/tex-index \
 && /usr/local/libexec/latex-core/tex-index build --bin-dir /opt/texlive/2026/bin/x86_64-linux --output /opt/latex-core/tex-environment-index.json \
 && rm -rf /tmp/par-* \
 && fc-cache -f \
 && mkdir -p /opt/latex-core/biber-cache \
 && chown 10001:10001 /opt/latex-core/biber-cache \
 && chmod 0700 /opt/latex-core/biber-cache
USER 10001:10001
RUN PAR_GLOBAL_TMPDIR=/opt/latex-core/biber-cache /opt/texlive/2026/bin/x86_64-linux/biber --version
USER root
RUN cache_count="$(find /opt/latex-core/biber-cache -type d -name 'cache-*' | wc -l)" \
 && test "$cache_count" -eq 1 \
 && cache_path="$(find /opt/latex-core/biber-cache -type d -name 'cache-*' -print -quit)" \
 && test -n "$cache_path" \
 && ln -s "$cache_path" /opt/latex-core/biber-runtime
ENV PAR_GLOBAL_TEMP=/opt/latex-core/biber-runtime
USER 10001:10001
RUN cache_path="$(/opt/texlive/2026/bin/x86_64-linux/biber --cache)" \
 && case "$cache_path" in /opt/latex-core/biber-runtime|/opt/latex-core/biber-cache/*) ;; *) exit 1 ;; esac \
 && /opt/texlive/2026/bin/x86_64-linux/biber --version
USER root
RUN rm -f /opt/latex-core/biber-runtime/inc.lock \
 && ln -s /tmp/latex-core-biber-inc.lock /opt/latex-core/biber-runtime/inc.lock \
 && chown -R root:10001 /opt/latex-core/biber-cache \
 && chmod -R a-w,g+rX /opt/latex-core/biber-cache
USER 10001:10001
RUN test ! -w /opt/latex-core/biber-runtime \
 && cache_path="$(/opt/texlive/2026/bin/x86_64-linux/biber --cache)" \
 && case "$cache_path" in /opt/latex-core/biber-runtime|/opt/latex-core/biber-cache/*) ;; *) exit 1 ;; esac \
 && /opt/texlive/2026/bin/x86_64-linux/biber --version
WORKDIR /work
ENTRYPOINT ["/usr/local/bin/compile-entrypoint"]
