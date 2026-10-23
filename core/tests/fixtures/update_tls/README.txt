Synthetic localhost TLS fixtures for native update transport regression tests.
The server key is intentionally public test material, unrelated to user credentials.
The fixture CA is trusted only by an explicitly constructed test agent. Production
uses the operating system verifier and cannot select this CA through the update API.
Certificates expire ten years after generation; renew these fixtures before then.
