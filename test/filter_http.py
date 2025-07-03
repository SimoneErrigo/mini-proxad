from proxad import HttpFlow, HttpResp, HttpReq


# Gets executed on every request / response pair
def http_filter(flow: HttpFlow, req: HttpReq, resp: HttpResp):
    print(flow)
    print(req)
    print(resp)

    if b"FLAG" in resp.body:
        body = resp.body.replace(b"FLAG", b"skibidi")
        return HttpResp(resp.headers, body, resp.status)

    resp.body = resp.body.replace(b"world", b"skibidi")
    return resp


# Gets executed everytime a flow is opened
# def http_open(flow):
#    print(flow)
