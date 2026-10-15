from calc import mean, median

def test_mean():
    assert mean([2, 4, 6]) == 4

def test_median_odd():
    assert median([1, 3, 2]) == 2

def test_median_single():
    assert median([7]) == 7
