"""Public HTTPS ingress for Standard Lab (browser only) and Pro Sunshine web UI.

Does not join volunteer hosts. Resolves a scoped Matchmaker token to one
session's mesh locator and reverse-proxies. Wrong/expired tokens fail closed.
"""
