import json
import httpx
from fastapi import FastAPI, Request, Response

app = FastAPI()

# Point this to your actual deepseek-npu container port on host
TARGET_BACKEND = "http://0.0.0.0:8001"

@app.api_route("/{path:path}", methods=["GET", "POST", "PUT", "DELETE"])
async def handle_proxy(request: Request, path: str):
    body = await request.body()
    headers = {k: v for k, v in request.headers.items() if k.lower() != "host"}

    if request.method == "POST" and "chat/completions" in path:
        try:
            data = json.loads(body)
            if "messages" in data:
                for msg in data["messages"]:
                    content = msg.get("content")
                    # If content is passed as an array of dicts, flatten to plain text string
                    if isinstance(content, list):
                        text_parts = [
                            item.get("text", "")
                            for item in content
                            if isinstance(item, dict) and item.get("type") == "text"
                        ]
                        msg["content"] = "\n".join(text_parts)

            body = json.dumps(data).encode("utf-8")
            headers["content-length"] = str(len(body))
        except Exception:
            pass  # Fall back to passing raw body if parsing fails

    async with httpx.AsyncClient() as client:
        backend_resp = await client.request(
            method=request.method,
            url=f"{TARGET_BACKEND}/{path}",
            headers=headers,
            content=body,
            timeout=120.0
        )
        return Response(
            content=backend_resp.content,
            status_code=backend_resp.status_code,
            headers=dict(backend_resp.headers)
        )
