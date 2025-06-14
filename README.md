# fluxrpc

> transport invariant RPC calls

---

## Protocol

**Event**

An event is defined by `event` and `data` property.
`event` is the name of the event and `data` is the events payload.

```json
{
    "event": "my_custom_event",
    "data": {
        "message": "data can be any payload"
    }
}
```

**Request**

A request must have `id`, `method` and `params` properties.
`params` can be any valid json

```json
{
    "id": "req_1234",
    "method": "add",
    "params": [1, 2, 3]
}
```

```json
{
    "id": "req_1234",
    "method": "print",
    "params": {"text": "Hello world", "color": false}
}
```

**Response**

```json
{
    "id": "req_1234",
    "result": "Hello world"
}
```

```json
{
    "id": "req_1234",
    "error": {
        "code": 403,
        "message": "Resource access forbidden",
        "data": {
            "username": "you@company.org",
            "roles": ["user"],
            "required_roles": ["manager"]
        }
    }
```
