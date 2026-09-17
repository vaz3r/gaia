FROM alpine:3.19@sha256:6baf43584bcb78f2e5847d1de515f23499913ac9f12bdf834811a3145eb11ca1

RUN apk add --no-cache wireguard-tools iptables iproute2 bash curl ca-certificates

ARG WSTUNNEL_VERSION=10.7.1
ARG WSTUNNEL_SHA256=fa842ed53fbb14b1c69cd98829f9895d7f8a6b0d562c57c1175851a52cea9ea2

RUN curl -sSL -o /tmp/wstunnel.tar.gz "https://github.com/erebe/wstunnel/releases/download/v${WSTUNNEL_VERSION}/wstunnel_${WSTUNNEL_VERSION}_linux_amd64.tar.gz" \
    && echo "${WSTUNNEL_SHA256}  /tmp/wstunnel.tar.gz" | sha256sum -c - \
    && tar -xz -C /usr/local/bin -f /tmp/wstunnel.tar.gz wstunnel \
    && rm -f /tmp/wstunnel.tar.gz \
    && chmod +x /usr/local/bin/wstunnel

COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
