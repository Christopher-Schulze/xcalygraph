(defpackage :util (:use :cl) (:export #:helper))
(defpackage :store (:use :cl :util) (:export #:open-store))
