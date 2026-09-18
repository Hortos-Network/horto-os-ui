FROM ghcr.io/seeed-projects/recomputer-rk-cv/rk3588-whisper:latest

WORKDIR /app

# Copy from the local wyoming subfolder
COPY wyoming/requirements.txt /app/requirements.txt

# Install dependencies
RUN python -m pip install --upgrade pip
RUN python -m pip install --no-cache-dir -r /app/requirements.txt

# Copy your script from the local wyoming subfolder
COPY wyoming/whisper_wyoming.py /app/whisper_wyoming.py

EXPOSE 10300

CMD ["python", "/app/whisper_wyoming.py", "--uri", "tcp://0.0.0.0:10300", "--model-dir", "/app/model"]
