*** Settings ***
Resource    ../resources/common.resource

*** Test Cases ***
Store Opens
    Prepare Store
    Check Store

*** Keywords ***
Check Store
    Log    ok
