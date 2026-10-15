from .util import normalize, missing_helper  # ImportError

def run(user):
    return normalize(user["name"])
