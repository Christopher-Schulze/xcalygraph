def FormData(value):
    return {"payload": value}

def env():
    return {"mode": "test"}

def onEvent(value):
    return [value]

def render(value):
    return {"rendered": value}

def test_payload():
    assert FormData("example") == {"payload": "example"}
