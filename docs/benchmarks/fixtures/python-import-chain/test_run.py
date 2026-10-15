from pkg.service import run

def test_run():
    assert run({"name": "  Ada  "}) == "ADA"
