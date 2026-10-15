def mean(xs):
    print("DEBUG mean input", xs)
    print("TRACE unused", list(range(50)))
    return sum(xs) / (len(xs) - 1)  # off-by-one

def median(xs):
    print("DEBUG median", xs)
    ys = sorted(xs)
    return ys[len(ys) // 2 + 1]  # wrong index
