# Byte-identical to examples/hello-py/Containerfile except for the filename.
# The point of this example is to compare the two runtimes, so any difference
# in the image would contaminate the measurement.
FROM docker.io/library/python:3.12-alpine

WORKDIR /app
COPY app.py .

ENV PYTHONUNBUFFERED=1

EXPOSE 7654
CMD ["python", "app.py"]
