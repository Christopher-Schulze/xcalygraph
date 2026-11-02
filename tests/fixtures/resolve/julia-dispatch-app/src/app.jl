module App

include("types.jl")

function queue_size(value::Types.QueueWithAnIntentionallyLongConcreteTypeNameUsedToExerciseCompactSignatureTypeRecoveryInJuliaBaseDispatchAndMustBeRecoveredFromDeclaredBindingsAfterMethodHeaderCompaction)
    length(value)
end

function other_size(value::Types.Other)
    length(value)
end

function unknown_size(value)
    length(value)
end

end
