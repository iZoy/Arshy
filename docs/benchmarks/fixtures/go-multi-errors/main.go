package main

import "fmt"

func add(a int, b int) int {
	return a + b
}

func main() {
	var x int = "nope" // type error
	fmt.Println(add(1, "2")) // type error
	fmt.Println(undefinedName)
	_ = x
}
