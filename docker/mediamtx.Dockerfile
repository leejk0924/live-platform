# The upstream image is scratch, so a healthcheck has nothing to run.
# A static busybox installed only as `wget` gives it an HTTP probe without a shell.
FROM busybox:1.37.0-musl@sha256:f6a95d611e0a8780fabdb2aec43e31275b2720dfefa3e41b16835bfc42454478 AS busybox
# The recording spool's mount point. Docker fills a new named volume from the image's
# directory, owner included, so the non-root user can write to it.
RUN mkdir /recordings

FROM bluenviron/mediamtx:1.21.3@sha256:59e43552f3be43695b70c19424dc2a42fcec18075f88d91561a24ac5d70a586c
COPY --from=busybox /bin/busybox /bin/wget
COPY --from=busybox --chown=65532:65532 /recordings /recordings
COPY deploy/mediamtx/mediamtx.yml /mediamtx.yml
USER 65532:65532
