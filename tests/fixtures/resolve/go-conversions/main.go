package convert

import "example.com/convert/model"

type Local uint

func convert(x uint) {
    _ = Local(x)
    _ = model.Code(x)
    model.Make(x)
}
