*** Settings ***
Resource    keywords/common.resource
Library     StoreLibrary.py

*** Test Cases ***
Open Store Works
    Open Store    path=x
    Should Be Equal    ${1}    ${1}

*** Keywords ***
Open Store
    [Arguments]    ${path}
    Log    ${path}
    Helper Keyword
