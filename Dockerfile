# Official Debian Bookworm slim OCI index, including native amd64 and arm64/v8.
# Source: https://hub.docker.com/v2/repositories/library/debian/tags/bookworm-slim
FROM docker.io/library/debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171

# BuildKit maps linux/amd64 and linux/arm64 to the verified unpacked payloads.
# CI unpacks this run's verified candidate archive in full; never compile here.
ARG TARGETARCH
ARG OCI_SOURCE
ARG OCI_REVISION
ARG OCI_VERSION
LABEL org.opencontainers.image.source="${OCI_SOURCE}" \
      org.opencontainers.image.revision="${OCI_REVISION}" \
      org.opencontainers.image.version="${OCI_VERSION}"
COPY release/${TARGETARCH}/ /opt/regen/

ENV PATH="/opt/regen:${PATH}"
WORKDIR /site
USER 65532:65532
ENTRYPOINT ["regen"]
CMD ["build", "--site", "/site"]
