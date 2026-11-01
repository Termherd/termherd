# termherd from a git ref (REPO for a fork); Rust from that ref's toolchain file.
FROM debian:bookworm@sha256:d74df7598108a173a621eda4d58a2adbf850b2d9a9d81bc2cca1a6c21174a880
ARG REPO=https://github.com/Termherd/termherd
ARG REF=7791bf5b9848791c45c7199e5ad863d6701361b7
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git build-essential pkg-config \
      libxkbcommon-dev libwayland-dev libxkbcommon-x11-dev libgl1-mesa-dev \
      libfontconfig1-dev libfreetype-dev \
      xvfb xdotool x11-apps imagemagick procps smem time bc jq \
      mesa-vulkan-drivers libvulkan1 libgl1-mesa-dri libegl1 fonts-dejavu \
      libx11-xcb1 libxcursor1 libxrandr2 libxi6 \
      && rm -rf /var/lib/apt/lists/*
RUN date +%s.%N > /bench-t0
RUN git clone "$REPO" /src && cd /src && git checkout --quiet "$REF" \
    && echo "termherd $REF $(git rev-parse HEAD)" > /bench-ref
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
ENV PATH=/root/.cargo/bin:$PATH
RUN cd /src && cargo build --release -p termherd-app && cp target/release/termherd /usr/local/bin/termherd
RUN date +%s.%N > /bench-t1
