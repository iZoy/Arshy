class User:
    is_active = False

def validate():
    user = User()
    assert user.is_active, "user must be active"

if __name__ == "__main__":
    validate()
