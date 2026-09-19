# Simulated Horto box: OpenSSH + sudo user (no Horto packages).
FROM debian:bookworm-slim

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        openssh-server sudo ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /var/run/sshd \
    && useradd -m -s /bin/bash horto \
    && echo 'horto:horto' | chpasswd \
    && echo 'horto ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/horto \
    && chmod 440 /etc/sudoers.d/horto \
    && mkdir -p /home/horto/.ssh \
    && chmod 700 /home/horto/.ssh \
    && chown -R horto:horto /home/horto/.ssh \
    && sed -i 's/#PasswordAuthentication yes/PasswordAuthentication yes/' /etc/ssh/sshd_config \
    && sed -i 's/#PubkeyAuthentication yes/PubkeyAuthentication yes/' /etc/ssh/sshd_config

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
