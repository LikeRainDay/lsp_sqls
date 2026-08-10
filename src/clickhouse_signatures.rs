use std::cmp::Ordering;
use std::sync::OnceLock;

use crate::builtin_signatures::BuiltinSignature;

// Generated from DBX's 2026-07-31 ClickHouse Playground snapshots in
// apps/desktop/src/lib/sql/clickhouse. Keep the compact name data static:
// completion scans names and constructs signatures only for bounded matches.
const REGULAR_NAMES: &str = r#"abs accurateCast accurateCastOrDefault accurateCastOrNull acos acosh addDate addDays addHours addInterval addMicroseconds addMilliseconds addMinutes addMonths addNanoseconds addQuarters addressToLine addressToLineWithInlines addressToSymbol addSeconds addTupleOfIntervals addWeeks addYears aes_decrypt_mysql aes_encrypt_mysql age aiClassify aiEmbed aiExtract aiGenerate aiTranslate alphaTokens and appendTrailingCharIfAbsent areaCartesian areaSpherical array arrayAll arrayAUCPR arrayAutocorrelation arrayAvg arrayBottomK arrayCompact arrayConcat arrayCount arrayCumSum arrayCumSumNonNegative arrayDifference arrayDistinct arrayDotProduct arrayElement arrayElementOrNull arrayEnumerate arrayEnumerateDense arrayEnumerateDenseRanked arrayEnumerateUniq arrayEnumerateUniqRanked arrayExcept arrayExists arrayFill arrayFilter arrayFirst arrayFirstIndex arrayFirstOrNull arrayFlatten arrayFold arrayIntersect arrayJaccardIndex arrayJoin arrayLast arrayLastIndex arrayLastOrNull arrayLevenshteinDistance arrayLevenshteinDistanceWeighted arrayMap arrayMax arrayMin arrayNormalizedGini arrayPartialReverseSort arrayPartialShuffle arrayPartialSort arrayPopBack arrayPopFront arrayProduct arrayPushBack arrayPushFront arrayRandomSample arrayReduce arrayReduceInRanges arrayRemove arrayResize arrayReverse arrayReverseFill arrayReverseSort arrayReverseSplit arrayROCAUC arrayRotateLeft arrayRotateRight arrayShiftLeft arrayShiftRight arrayShingles arrayShuffle arraySimilarity arraySlice arraySort arraySplit arrayStringConcat arraySum arraySymmetricDifference arrayTopK arrayTranspose arrayUnion arrayUniq arrayWithConstant arrayZip arrayZipUnaligned ascii asin asinh assumeNotNull atan atan2 atanh authenticatedUser avg2 bar base32Decode base32Encode base58Decode base58Encode base64Decode base64Encode base64URLDecode base64URLEncode basename bech32Decode bech32Encode bin bitAnd bitCount bitHammingDistance bitmapAnd bitmapAndCardinality bitmapAndnot bitmapAndnotCardinality bitmapBuild bitmapCardinality bitmapContains bitmapHasAll bitmapHasAny bitmapMax bitmapMin bitmapOr bitmapOrCardinality bitmapSubsetInRange bitmapSubsetLimit bitmapToArray bitmapTransform bitmapXor bitmapXorCardinality bitmaskToArray bitmaskToList bitNot bitOr bitPositionsToArray bitRotateLeft bitRotateRight bitShiftLeft bitShiftRight bitSlice bitTest bitTestAll bitTestAny bitXor BLAKE3 blockNumber blockSerializedSize blockSize buildId byteHammingDistance byteSize byteSwap caseFoldUTF8 caseWithExpression CAST catboostEvaluate cbrt ceil changeDay changeHour changeMinute changeMonth changeSecond changeYear char cityHash64 clamp coalesce colorOKLABToSRGB colorOKLCHToSRGB colorSRGBToOKLAB colorSRGBToOKLCH compareSubstrings concat concatAssumeInjective concatWithSeparator concatWithSeparatorAssumeInjective connectionId conv convertCharset cos cosh cosineDistance cosineDistanceTransposed countDigits countEqual countMatches countMatchesCaseInsensitive countSubstrings countSubstringsCaseInsensitive countSubstringsCaseInsensitiveUTF8 CRC32 CRC32IEEE CRC64 cume_dist currentDatabase currentProfiles currentQueryID currentRoles currentSchemas currentUser cutFragment cutIPv6 cutQueryString cutQueryStringAndFragment cutToFirstSignificantSubdomain cutToFirstSignificantSubdomainCustom cutToFirstSignificantSubdomainCustomRFC cutToFirstSignificantSubdomainCustomWithWWW cutToFirstSignificantSubdomainCustomWithWWWRFC cutToFirstSignificantSubdomainRFC cutToFirstSignificantSubdomainWithWWW cutToFirstSignificantSubdomainWithWWWRFC cutURLParameter cutWWW damerauLevenshteinDistance DATE dateDiff dateName dateTime64ToSnowflakeID dateTimeToSnowflakeID dateTimeToUUIDv7 dateTrunc decodeHTMLComponent decodeURLComponent decodeURLFormComponent decodeXMLComponent decrypt defaultProfiles defaultRoles defaultValueOfArgumentType defaultValueOfTypeName degrees demangle dense_rank dequantizeInt8ToBFloat16 detectCharset detectLanguage detectLanguageMixed detectLanguageUnknown detectTonality dictGet dictGetAll dictGetChildren dictGetDate dictGetDateOrDefault dictGetDateTime dictGetDateTimeOrDefault dictGetDescendants dictGetFloat32 dictGetFloat32OrDefault dictGetFloat64 dictGetFloat64OrDefault dictGetHierarchy dictGetInt16 dictGetInt16OrDefault dictGetInt32 dictGetInt32OrDefault dictGetInt64 dictGetInt64OrDefault dictGetInt8 dictGetInt8OrDefault dictGetIPv4 dictGetIPv4OrDefault dictGetIPv6 dictGetIPv6OrDefault dictGetKeys dictGetOrDefault dictGetOrNull dictGetString dictGetStringOrDefault dictGetUInt16 dictGetUInt16OrDefault dictGetUInt32 dictGetUInt32OrDefault dictGetUInt64 dictGetUInt64OrDefault dictGetUInt8 dictGetUInt8OrDefault dictGetUUID dictGetUUIDOrDefault dictHas dictIsIn displayName divide divideDecimal divideOrNull domain domainRFC domainWithoutWWW domainWithoutWWWRFC dotProduct dotProductTransposed dumpColumnStructure dynamicElement dynamicType e editDistance editDistanceUTF8 empty emptyArrayDate emptyArrayDateTime emptyArrayFloat32 emptyArrayFloat64 emptyArrayInt16 emptyArrayInt32 emptyArrayInt64 emptyArrayInt8 emptyArrayString emptyArrayToSingle emptyArrayUInt16 emptyArrayUInt32 emptyArrayUInt64 emptyArrayUInt8 enabledProfiles enabledRoles encodeURLComponent encodeURLFormComponent encodeXMLComponent encrypt endsWith endsWithCaseInsensitive endsWithCaseInsensitiveUTF8 endsWithUTF8 equals erf erfc errorCodeToName evalMLMethod exp exp10 exp2 extract extractAll extractAllGroupsHorizontal extractAllGroupsVertical extractGroups extractKeyValuePairs extractKeyValuePairsWithEscaping extractTextFromHTML extractURLParameter extractURLParameterNames extractURLParameters factorial farmFingerprint64 farmHash64 filesystemAvailable filesystemCapacity filesystemUnreserved finalizeAggregation financialInternalRateOfReturn financialInternalRateOfReturnExtended financialNetPresentValue financialNetPresentValueExtended first_value firstLine firstNonDefault firstSignificantSubdomain firstSignificantSubdomainCustom firstSignificantSubdomainCustomRFC firstSignificantSubdomainRFC flattenTuple flipCoordinates floor formatDateTime formatDateTimeInJodaSyntax formatQuery formatQueryOrNull formatQuerySingleLine formatQuerySingleLineOrNull formatReadableDecimalSize formatReadableQuantity formatReadableSize formatReadableTimeDelta formatRow formatRowNoNewline FQDN fragment fromDaysSinceYearZero fromDaysSinceYearZero32 fromModifiedJulianDay fromModifiedJulianDayOrNull fromUnixTimestamp fromUnixTimestamp64Micro fromUnixTimestamp64Milli fromUnixTimestamp64Nano fromUnixTimestamp64Second fromUnixTimestampInJodaSyntax fromUTCTimestamp fuzzBits gccMurmurHash gcd generateRandomStructure generateSerialID generateSnowflakeID generateULID generateUUIDv4 generateUUIDv7 geoDistance geohashDecode geohashEncode geohashesInBox geoToH3 geoToS2 getClientHTTPHeader getMacro getMaxTableNameLengthForDatabase getMergeTreeSetting getOSKernelVersion getServerPort getServerSetting getSetting getSettingOrDefault getSizeOfEnumType getSubcolumn getTypeSerializationStreams globalIn globalInIgnoreSet globalNotIn globalNotInIgnoreSet globalNotNullIn globalNotNullInIgnoreSet globalNullIn globalNullInIgnoreSet globalVariable greatCircleAngle greatCircleDistance greater greaterOrEquals greatest h3CellAreaM2 h3CellAreaRads2 h3Distance h3EdgeAngle h3EdgeLengthKm h3EdgeLengthM h3ExactEdgeLengthKm h3ExactEdgeLengthM h3ExactEdgeLengthRads h3GetBaseCell h3GetDestinationIndexFromUnidirectionalEdge h3GetFaces h3GetIndexesFromUnidirectionalEdge h3GetOriginIndexFromUnidirectionalEdge h3GetPentagonIndexes h3GetRes0Indexes h3GetResolution h3GetUnidirectionalEdge h3GetUnidirectionalEdgeBoundary h3GetUnidirectionalEdgesFromHexagon h3HexAreaKm2 h3HexAreaM2 h3HexRing h3IndexesAreNeighbors h3IsPentagon h3IsResClassIII h3IsValid h3kRing h3Line h3NumHexagons h3PointDistKm h3PointDistM h3PointDistRads h3PolygonToCells h3PolygonToCellsWithContainment h3ToCenterChild h3ToChildren h3ToGeo h3ToGeoBoundary h3ToParent h3ToString h3UnidirectionalEdgeIsValid halfMD5 has hasAll hasAllTokens hasAny hasAnyTokens hasColumnInTable hasPhrase hasSubsequence hasSubsequenceCaseInsensitive hasSubsequenceCaseInsensitiveUTF8 hasSubsequenceUTF8 hasSubstr hasThreadFuzzer hasToken hasTokenCaseInsensitive hasTokenCaseInsensitiveOrNull hasTokenOrNull hex highlight highlightQuery hilbertDecode hilbertEncode hiveHash HMAC hop hopEnd hopStart hostName hypot icebergBucket icebergHash icebergTruncate identity idnaDecode idnaEncode if ifNotFinite ifNull ignore ilike in indexHint indexOf indexOfAssumeSorted inIgnoreSet initcap initcapUTF8 initializeAggregation initialQueryID initialQueryStartTime intDiv intDivOrNull intDivOrZero intExp10 intExp2 intHash32 intHash64 IPv4CIDRToRange IPv4NumToString IPv4NumToStringClassC IPv4StringToNum IPv4StringToNumOrDefault IPv4StringToNumOrNull IPv4ToIPv6 IPv6CIDRToRange IPv6NumToString IPv6StringToNum IPv6StringToNumOrDefault IPv6StringToNumOrNull isConstant isDecimalOverflow isDistinctFrom isDynamicElementInSharedData isFinite isInfinite isIPAddressInRange isIPv4String isIPv6String isMergeTreePartCoveredBy isNaN isNotDistinctFrom isNotNull isNull isNullable isPrime isProbablePrime isValidASCII isValidJSON isValidUTF8 isZeroOrNull jaroSimilarity jaroWinklerSimilarity javaHash javaHashUTF16LE joinGet joinGetOrNull JSON_EXISTS JSON_QUERY JSON_VALUE JSONAllPaths JSONAllPathsWithTypes JSONAllValues JSONArrayLength JSONDynamicPaths JSONDynamicPathsWithTypes JSONExtract JSONExtractArrayRaw JSONExtractArrayRawCaseInsensitive JSONExtractBool JSONExtractBoolCaseInsensitive JSONExtractCaseInsensitive JSONExtractFloat JSONExtractFloatCaseInsensitive JSONExtractInt JSONExtractIntCaseInsensitive JSONExtractKeys JSONExtractKeysAndValues JSONExtractKeysAndValuesCaseInsensitive JSONExtractKeysAndValuesRaw JSONExtractKeysAndValuesRawCaseInsensitive JSONExtractKeysCaseInsensitive JSONExtractRaw JSONExtractRawCaseInsensitive JSONExtractString JSONExtractStringCaseInsensitive JSONExtractUInt JSONExtractUIntCaseInsensitive JSONHas JSONKey JSONLength JSONMergePatch JSONSharedDataPaths JSONSharedDataPathsWithTypes JSONType jumpConsistentHash kafkaMurmurHash keccak256 kostikConsistentHash L1Distance L1Norm L1Normalize L2Distance L2DistanceTransposed L2Norm L2Normalize L2SquaredDistance L2SquaredNorm lag lagInFrame last_value lcm lead leadInFrame least left leftPad leftPadUTF8 leftUTF8 lemmatize length lengthUTF8 less lessOrEquals lgamma like LinfDistance LinfNorm LinfNormalize localtime locate log log10 log1p log2 logTrace lowCardinalityIndices lowCardinalityKeys lower lowerUTF8 LpDistance LpNorm LpNormalize MACNumToString MACStringToNum MACStringToOUI makeDate makeDate32 makeDateTime makeDateTime64 map mapAdd mapAll mapApply mapConcat mapContainsKey mapContainsKeyLike mapContainsValue mapContainsValueLike mapExists mapExtractKeyLike mapExtractValueLike mapFilter mapFromArrays mapKeys mapPartialReverseSort mapPartialSort mapPopulateSeries mapReverseSort mapSort mapSubtract mapUpdate mapValues match materialize max2 MD4 MD5 mergeTreePartInfo metroHash64 midpoint min2 minSampleSizeContinuous minSampleSizeConversion minus modulo moduloLegacy moduloOrNull moduloOrZero monthName mortonDecode mortonEncode multiFuzzyMatchAllIndices multiFuzzyMatchAny multiFuzzyMatchAnyIndex multiIf multiMatchAllIndices multiMatchAny multiMatchAnyIndex multiply multiplyDecimal multiSearchAllPositions multiSearchAllPositionsCaseInsensitive multiSearchAllPositionsCaseInsensitiveUTF8 multiSearchAllPositionsUTF8 multiSearchAny multiSearchAnyCaseInsensitive multiSearchAnyCaseInsensitiveUTF8 multiSearchAnyUTF8 multiSearchFirstIndex multiSearchFirstIndexCaseInsensitive multiSearchFirstIndexCaseInsensitiveUTF8 multiSearchFirstIndexUTF8 multiSearchFirstPosition multiSearchFirstPositionCaseInsensitive multiSearchFirstPositionCaseInsensitiveUTF8 multiSearchFirstPositionUTF8 murmurHash2_32 murmurHash2_64 murmurHash3_128 murmurHash3_32 murmurHash3_64 MVTBoundingBox MVTBoundingBoxMercator MVTEncodeGeom naiveBayesClassifier naturalSortKey negate neighbor nested netloc ngramDistance ngramDistanceCaseInsensitive ngramDistanceCaseInsensitiveUTF8 ngramDistanceUTF8 ngramMinHash ngramMinHashArg ngramMinHashArgCaseInsensitive ngramMinHashArgCaseInsensitiveUTF8 ngramMinHashArgUTF8 ngramMinHashCaseInsensitive ngramMinHashCaseInsensitiveUTF8 ngramMinHashUTF8 ngrams ngramSearch ngramSearchCaseInsensitive ngramSearchCaseInsensitiveUTF8 ngramSearchUTF8 ngramSimHash ngramSimHashCaseInsensitive ngramSimHashCaseInsensitiveUTF8 ngramSimHashUTF8 normalizedQueryHash normalizedQueryHashKeepNames normalizeQuery normalizeQueryKeepNames normalizeUTF8NFC normalizeUTF8NFD normalizeUTF8NFKC normalizeUTF8NFKCCasefold normalizeUTF8NFKD not notEmpty notEquals notILike notIn notInIgnoreSet notLike notNullIn notNullInIgnoreSet now now64 nowInBlock nowInBlock64 nth_value ntile nullIf nullIn nullInIgnoreSet numericIndexedVectorAllValueSum numericIndexedVectorBuild numericIndexedVectorCardinality numericIndexedVectorGetValue numericIndexedVectorPointwiseAdd numericIndexedVectorPointwiseDivide numericIndexedVectorPointwiseEqual numericIndexedVectorPointwiseGreater numericIndexedVectorPointwiseGreaterEqual numericIndexedVectorPointwiseLess numericIndexedVectorPointwiseLessEqual numericIndexedVectorPointwiseMultiply numericIndexedVectorPointwiseNotEqual numericIndexedVectorPointwiseSubtract numericIndexedVectorShortDebugString numericIndexedVectorToMap obfuscateQuery obfuscateQueryWithSeed or overlay overlayUTF8 parseDateTime parseDateTime32BestEffort parseDateTime32BestEffortOrNull parseDateTime32BestEffortOrZero parseDateTime64 parseDateTime64BestEffort parseDateTime64BestEffortOrNull parseDateTime64BestEffortOrZero parseDateTime64BestEffortUS parseDateTime64BestEffortUSOrNull parseDateTime64BestEffortUSOrZero parseDateTime64InJodaSyntax parseDateTime64InJodaSyntaxOrNull parseDateTime64InJodaSyntaxOrZero parseDateTime64OrNull parseDateTime64OrZero parseDateTimeBestEffort parseDateTimeBestEffortOrNull parseDateTimeBestEffortOrZero parseDateTimeBestEffortUS parseDateTimeBestEffortUSOrNull parseDateTimeBestEffortUSOrZero parseDateTimeInJodaSyntax parseDateTimeInJodaSyntaxOrNull parseDateTimeInJodaSyntaxOrZero parseDateTimeOrNull parseDateTimeOrZero parseReadableSize parseReadableSizeOrNull parseReadableSizeOrZero parseTimeDelta partitionId path pathFull percent_rank perimeterCartesian perimeterSpherical pi plus pointInEllipses pointInPolygon polygonAreaCartesian polygonAreaSpherical polygonConvexHullCartesian polygonPerimeterCartesian polygonPerimeterSpherical polygonsDistanceCartesian polygonsDistanceSpherical polygonsEqualsCartesian polygonsIntersectCartesian polygonsIntersectionCartesian polygonsIntersectionSpherical polygonsIntersectSpherical polygonsSymDifferenceCartesian polygonsSymDifferenceSpherical polygonsUnionCartesian polygonsUnionSpherical polygonsWithinCartesian polygonsWithinSpherical port portRFC position positionCaseInsensitive positionCaseInsensitiveUTF8 positionUTF8 positiveModulo positiveModuloOrNull pow prettyPrintJSON printf proportionsZTest protocol punycodeDecode punycodeEncode quantizeBFloat16ToInt8 queryID queryString queryStringAndFragment radians rand rand64 randBernoulli randBinomial randCanonical randChiSquared randConstant randExponential randFisherF randLogNormal randNegativeBinomial randNormal randomFixedString randomHadamardTransform randomPrintableASCII randomString randomStringUTF8 randPoisson randStudentT randUniform range rank readWKB readWKBLineString readWKBMultiLineString readWKBMultiPolygon readWKBPoint readWKBPolygon readWKT readWKTLineString readWKTMultiLineString readWKTMultiPolygon readWKTPoint readWKTPolygon readWKTRing regexpExtract regexpPosition regexpQuoteMeta regionHierarchy regionIn regionToArea regionToCity regionToContinent regionToCountry regionToDistrict regionToName regionToPopulation regionToTopContinent reinterpret reinterpretAsDate reinterpretAsDateTime reinterpretAsFixedString reinterpretAsFloat32 reinterpretAsFloat64 reinterpretAsInt128 reinterpretAsInt16 reinterpretAsInt256 reinterpretAsInt32 reinterpretAsInt64 reinterpretAsInt8 reinterpretAsString reinterpretAsUInt128 reinterpretAsUInt16 reinterpretAsUInt256 reinterpretAsUInt32 reinterpretAsUInt64 reinterpretAsUInt8 reinterpretAsUUID removeDiacriticsUTF8 repeat replaceAll replaceOne replaceRegexpAll replaceRegexpOne replicate reverse reverseBySeparator reverseUTF8 revision right rightPad rightPadUTF8 rightUTF8 RIPEMD160 round roundAge roundBankers roundDown roundDuration roundToExp2 row_number rowNumberInAllBlocks rowNumberInBlock runningAccumulate runningConcurrency runningDifference runningDifferenceStartingWithFirstValue s2CapContains s2CapUnion s2CellsIntersect s2GetNeighbors s2RectAdd s2RectContains s2RectIntersection s2RectUnion s2ToGeo seriesDecomposeSTL seriesOutliersDetectTukey seriesPeriodDetectFFT serverTimezone serverUUID SHA1 SHA224 SHA256 SHA384 SHA512 SHA512_256 shardCount shardNum showCertificate sigmoid sign simpleJSONExtractBool simpleJSONExtractFloat simpleJSONExtractInt simpleJSONExtractRaw simpleJSONExtractString simpleJSONExtractUInt simpleJSONHas sin sinh sipHash128 sipHash128Keyed sipHash128Reference sipHash128ReferenceKeyed sipHash64 sipHash64Keyed sleep sleepEachRow snowflakeIDToDateTime snowflakeIDToDateTime64 soundex space sparseGrams sparseGramsHashes sparseGramsHashesUTF8 sparseGramsUTF8 splitByChar splitByNonAlpha splitByRegexp splitByString splitByWhitespace sqidDecode sqidEncode sqrt startsWith startsWithCaseInsensitive startsWithCaseInsensitiveUTF8 startsWithUTF8 stem stringBytesEntropy stringBytesUniq stringJaccardIndex stringJaccardIndexUTF8 stringToH3 structureToCapnProtoSchema structureToProtobufSchema subBitmap subDate substring substringIndex substringIndexUTF8 substringUTF8 subtractDays subtractHours subtractInterval subtractMicroseconds subtractMilliseconds subtractMinutes subtractMonths subtractNanoseconds subtractQuarters subtractSeconds subtractTupleOfIntervals subtractWeeks subtractYears svg synonyms tan tanh tcpPort tgamma throwIf tid timeDiff timeSeriesCopyTag timeSeriesCopyTags timeSeriesExtractTag timeSeriesFromGrid timeSeriesGroupToSamplingKey timeSeriesGroupToTags timeSeriesIdToGroup timeSeriesIdToTags timeSeriesJoinTags timeSeriesRange timeSeriesRemoveAllTagsExcept timeSeriesRemoveTag timeSeriesRemoveTags timeSeriesReplaceTag timeSeriesStoreTags timeSeriesTagsToGroup timeSeriesThrowDuplicateSeriesIf timeSlot timeSlots timestamp timezone timezoneOf timezoneOffset toBFloat16 toBFloat16OrNull toBFloat16OrZero toBool toColumnTypeName toDate toDate32 toDate32OrDefault toDate32OrNull toDate32OrZero toDateOrDefault toDateOrNull toDateOrZero toDateTime toDateTime32 toDateTime64 toDateTime64OrDefault toDateTime64OrNull toDateTime64OrZero toDateTimeOrDefault toDateTimeOrNull toDateTimeOrZero today toDayOfMonth toDayOfWeek toDayOfYear toDaysInMonth toDaysSinceYearZero toDecimal128 toDecimal128OrDefault toDecimal128OrNull toDecimal128OrZero toDecimal256 toDecimal256OrDefault toDecimal256OrNull toDecimal256OrZero toDecimal32 toDecimal32OrDefault toDecimal32OrNull toDecimal32OrZero toDecimal64 toDecimal64OrDefault toDecimal64OrNull toDecimal64OrZero toDecimalString toFixedString toFloat32 toFloat32OrDefault toFloat32OrNull toFloat32OrZero toFloat64 toFloat64OrDefault toFloat64OrNull toFloat64OrZero toHour toInt128 toInt128OrDefault toInt128OrNull toInt128OrZero toInt16 toInt16OrDefault toInt16OrNull toInt16OrZero toInt256 toInt256OrDefault toInt256OrNull toInt256OrZero toInt32 toInt32OrDefault toInt32OrNull toInt32OrZero toInt64 toInt64OrDefault toInt64OrNull toInt64OrZero toInt8 toInt8OrDefault toInt8OrNull toInt8OrZero toInterval toIntervalDay toIntervalHour toIntervalMicrosecond toIntervalMillisecond toIntervalMinute toIntervalMonth toIntervalNanosecond toIntervalQuarter toIntervalSecond toIntervalWeek toIntervalYear toIPv4 toIPv4OrDefault toIPv4OrNull toIPv4OrZero toIPv6 toIPv6OrDefault toIPv6OrNull toIPv6OrZero toISOWeek toISOYear toJSONString tokenizeQuery tokens tokensForLikePattern toLastDayOfMonth toLastDayOfWeek toLowCardinality toMicrosecond toMillisecond toMinute toModifiedJulianDay toModifiedJulianDayOrNull toMonday toMonth toMonthNumSinceEpoch toNanosecond toNullable topLevelDomain topLevelDomainRFC toQuarter toRelativeDayNum toRelativeHourNum toRelativeMinuteNum toRelativeMonthNum toRelativeQuarterNum toRelativeSecondNum toRelativeWeekNum toRelativeYearNum toSecond toStartOfDay toStartOfFifteenMinutes toStartOfFiveMinutes toStartOfHour toStartOfInterval toStartOfISOYear toStartOfMicrosecond toStartOfMillisecond toStartOfMinute toStartOfMonth toStartOfNanosecond toStartOfQuarter toStartOfSecond toStartOfTenMinutes toStartOfWeek toStartOfYear toString toStringCutToZero toTime toTime64 toTime64OrNull toTime64OrZero toTimeOrNull toTimeOrZero toTimeWithFixedDate toTimezone toTypeName toUInt128 toUInt128OrDefault toUInt128OrNull toUInt128OrZero toUInt16 toUInt16OrDefault toUInt16OrNull toUInt16OrZero toUInt256 toUInt256OrDefault toUInt256OrNull toUInt256OrZero toUInt32 toUInt32OrDefault toUInt32OrNull toUInt32OrZero toUInt64 toUInt64OrDefault toUInt64OrNull toUInt64OrZero toUInt8 toUInt8OrDefault toUInt8OrNull toUInt8OrZero toUnixTimestamp toUnixTimestamp64Micro toUnixTimestamp64Milli toUnixTimestamp64Nano toUnixTimestamp64Second toUTCTimestamp toUUID toUUIDOrDefault toUUIDOrNull toUUIDOrZero toValidUTF8 toWeek toYear toYearNumSinceEpoch toYearWeek toYYYYMM toYYYYMMDD toYYYYMMDDhhmmss transactionID transactionLatestSnapshot transactionOldestSnapshot transform translate translateUTF8 trimBoth trimLeft trimRight trunc tryBase32Decode tryBase58Decode tryBase64Decode tryBase64URLDecode tryDecrypt tryIdnaEncode tryPunycodeDecode tumble tumbleEnd tumbleStart tuple tupleConcat tupleDivide tupleDivideByNumber tupleElement tupleHammingDistance tupleIntDiv tupleIntDivByNumber tupleIntDivOrZero tupleIntDivOrZeroByNumber tupleMinus tupleModulo tupleModuloByNumber tupleMultiply tupleMultiplyByNumber tupleNames tupleNegate tuplePlus tuplePositiveModuloByNumber tupleToNameValuePairs ULIDStringToDateTime unbin unhex uniqThetaIntersect uniqThetaNot uniqThetaUnion upper upperUTF8 uptime URLHash URLHierarchy URLPathHierarchy UTCTimestamp UUIDNumToString UUIDStringToNum UUIDToNum UUIDv7ToDateTime validateNestedArraySizes variantElement variantType version visibleWidth widthBucket windowID wkb wkt wordShingleMinHash wordShingleMinHashArg wordShingleMinHashArgCaseInsensitive wordShingleMinHashArgCaseInsensitiveUTF8 wordShingleMinHashArgUTF8 wordShingleMinHashCaseInsensitive wordShingleMinHashCaseInsensitiveUTF8 wordShingleMinHashUTF8 wordShingleSimHash wordShingleSimHashCaseInsensitive wordShingleSimHashCaseInsensitiveUTF8 wordShingleSimHashUTF8 wyHash64 xor xxh3 xxh3_128 xxHash32 xxHash64 yesterday YYYYMMDDhhmmssToDateTime YYYYMMDDhhmmssToDateTime64 YYYYMMDDToDate YYYYMMDDToDate32 zookeeperSessionUptime"#;
const AGGREGATE_NAMES: &str = r#"aggThrow analysisOfVariance any any_respect_nulls anyHeavy anyLast anyLast_respect_nulls approx_top_k approx_top_sum argAndMax argAndMin argMax argMin avg avgWeighted boundingRatio categoricalInformationValue contingency corr corrMatrix corrStable count covarPop covarPopMatrix covarPopStable covarSamp covarSampMatrix covarSampStable cramersV cramersVBiasCorrected deltaSum deltaSumTimestamp distinctDynamicTypes distinctJSONPaths distinctJSONPathsAndTypes entropy estimateCompressionRatio exponentialMovingAverage exponentialTimeDecayedAvg exponentialTimeDecayedCount exponentialTimeDecayedMax exponentialTimeDecayedSum flameGraph groupArray groupArrayInsertAt groupArrayIntersect groupArrayLast groupArrayMovingAvg groupArrayMovingSum groupArraySample groupArraySorted groupBitAnd groupBitmap groupBitmapAnd groupBitmapOr groupBitmapXor groupBitOr groupBitXor groupConcat groupFormat groupNumericIndexedVector groupUniqArray histogram intervalLengthSum kolmogorovSmirnovTest kurtPop kurtSamp largestTriangleThreeBuckets mannWhitneyUTest max maxIntersections maxIntersectionsPosition maxMappedArrays meanZTest min minMappedArrays MVTEncode nonNegativeDerivative nothing nothingNull nothingUInt64 quantile quantileBFloat16 quantileBFloat16Weighted quantileDD quantileDeterministic quantileExact quantileExactExclusive quantileExactHigh quantileExactInclusive quantileExactLow quantileExactWeighted quantileExactWeightedInterpolated quantileGK quantileInterpolatedWeighted quantilePrometheusHistogram quantiles quantilesBFloat16 quantilesBFloat16Weighted quantilesDD quantilesDeterministic quantilesExact quantilesExactExclusive quantilesExactHigh quantilesExactInclusive quantilesExactLow quantilesExactWeighted quantilesExactWeightedInterpolated quantilesGK quantilesInterpolatedWeighted quantilesPrometheusHistogram quantilesTDigest quantilesTDigestWeighted quantilesTiming quantilesTimingWeighted quantileTDigest quantileTDigestWeighted quantileTiming quantileTimingWeighted rankCorr retention sequenceCount sequenceMatch sequenceMatchEvents sequenceNextNode simpleLinearRegression singleValueOrNull skewPop skewSamp sparkbar stddevPop stddevPopStable stddevSamp stddevSampStable stochasticLinearRegression stochasticLogisticRegression studentTTest studentTTestOneSample sum sumCount sumKahan sumMapFiltered sumMapFilteredWithOverflow sumMappedArrays sumMapWithOverflow sumWithOverflow theilsU timeSeriesChangesToGrid timeSeriesDeltaToGrid timeSeriesDerivToGrid timeSeriesGroupArray timeSeriesInstantDeltaToGrid timeSeriesInstantRateToGrid timeSeriesLastTwoSamples timeSeriesPredictLinearToGrid timeSeriesRateToGrid timeSeriesResampleToGridWithStaleness timeSeriesResetsToGrid topK topKWeighted uniq uniqCombined uniqCombined64 uniqExact uniqHLL12 uniqTheta uniqUpTo varPop varPopStable varSamp varSampStable welchTTest windowFunnel"#;
const TABLE_NAMES: &str = r#"arrowFlight azureBlobStorage azureBlobStorageCluster cluster clusterAllReplicas cosn deltaLake deltaLakeAzure deltaLakeAzureCluster deltaLakeCluster deltaLakeLocal deltaLakeS3 deltaLakeS3Cluster dictionary executable file fileCluster filesystem format fuzzJSON fuzzQuery gcs generate_series generateRandom generateSeries hdfs hdfsCluster hive hudi hudiCluster iceberg icebergAzure icebergAzureCluster icebergCluster icebergHDFS icebergHDFSCluster icebergLocal icebergLocalCluster icebergS3 icebergS3Cluster input jdbc loop merge mergeTreeAnalyzeIndexes mergeTreeAnalyzeIndexesUUID mergeTreeIndex mergeTreeProjection mergeTreeTextIndex mongodb mysql null numbers numbers_mt odbc oss paimon paimonAzure paimonAzureCluster paimonCluster paimonHDFS paimonHDFSCluster paimonLocal paimonS3 paimonS3Cluster postgresql primes prometheusQuery prometheusQueryRange redis remote remoteSecure s3 s3Cluster sqlite SQLStandardValues timeSeriesData timeSeriesMetrics timeSeriesSamples timeSeriesSelector timeSeriesTags url urlCluster values view viewExplain viewIfPermitted ytsaurus zeros zeros_mt"#;

const ALIASES: &[(&str, &str)] = &[
    ("denseRank", "dense_rank"),
    ("splitByAlpha", "alphaTokens"),
    ("arrayPRAUC", "arrayAUCPR"),
    ("flatten", "arrayFlatten"),
    ("unnest", "arrayJoin"),
    ("array_remove", "arrayRemove"),
    ("arrayAUC", "arrayROCAUC"),
    ("array_to_string", "arrayStringConcat"),
    ("authUser", "authenticatedUser"),
    ("FROM_BASE64", "base64Decode"),
    ("TO_BASE64", "base64Encode"),
    ("mismatches", "byteHammingDistance"),
    ("caseWithExpr", "caseWithExpression"),
    ("ceiling", "ceil"),
    ("concat_ws", "concatWithSeparator"),
    ("connection_id", "connectionId"),
    ("distanceCosine", "cosineDistance"),
    ("distanceCosineTransposed", "cosineDistanceTransposed"),
    ("DATABASE", "currentDatabase"),
    ("SCHEMA", "currentDatabase"),
    ("current_database", "currentDatabase"),
    ("current_query_id", "currentQueryID"),
    ("current_schemas", "currentSchemas"),
    ("current_user", "currentUser"),
    ("session_user", "currentUser"),
    ("user", "currentUser"),
    ("DATE_DIFF", "dateDiff"),
    ("TIMESTAMP_DIFF", "dateDiff"),
    ("timestampDiff", "dateDiff"),
    ("DATE_TRUNC", "dateTrunc"),
    ("scalarProduct", "dotProduct"),
    ("scalarProductTransposed", "dotProductTransposed"),
    ("levenshteinDistance", "editDistance"),
    ("levenshteinDistanceUTF8", "editDistanceUTF8"),
    ("extractAllGroups", "extractAllGroupsVertical"),
    ("mapFromString", "extractKeyValuePairs"),
    ("str_to_map", "extractKeyValuePairs"),
    ("DATE_FORMAT", "formatDateTime"),
    ("FORMAT_BYTES", "formatReadableSize"),
    ("fullHostName", "FQDN"),
    ("FROM_DAYS", "fromDaysSinceYearZero"),
    ("FROM_UNIXTIME", "fromUnixTimestamp"),
    ("from_utc_timestamp", "fromUTCTimestamp"),
    ("hasAllToken", "hasAllTokens"),
    ("hasAnyToken", "hasAnyTokens"),
    ("matchPhrase", "hasPhrase"),
    ("initial_query_id", "initialQueryID"),
    ("initial_query_start_time", "initialQueryStartTime"),
    ("INET_NTOA", "IPv4NumToString"),
    ("INET_ATON", "IPv4StringToNum"),
    ("INET6_NTOA", "IPv6NumToString"),
    ("INET6_ATON", "IPv6StringToNum"),
    ("isASCII", "isValidASCII"),
    ("JSON_ARRAY_LENGTH", "JSONArrayLength"),
    ("yandexConsistentHash", "kostikConsistentHash"),
    ("distanceL1", "L1Distance"),
    ("normL1", "L1Norm"),
    ("normalizeL1", "L1Normalize"),
    ("distanceL2", "L2Distance"),
    ("distanceL2Transposed", "L2DistanceTransposed"),
    ("normL2", "L2Norm"),
    ("normalizeL2", "L2Normalize"),
    ("distanceL2Squared", "L2SquaredDistance"),
    ("normL2Squared", "L2SquaredNorm"),
    ("lpad", "leftPad"),
    ("CARDINALITY", "length"),
    ("OCTET_LENGTH", "length"),
    ("CHARACTER_LENGTH", "lengthUTF8"),
    ("CHAR_LENGTH", "lengthUTF8"),
    ("distanceLinf", "LinfDistance"),
    ("normLinf", "LinfNorm"),
    ("normalizeLinf", "LinfNormalize"),
    ("ln", "log"),
    ("lcase", "lower"),
    ("distanceLp", "LpDistance"),
    ("normLp", "LpNorm"),
    ("normalizeLp", "LpNormalize"),
    ("mapContains", "mapContainsKey"),
    ("MAP_FROM_ARRAYS", "mapFromArrays"),
    ("REGEXP_MATCHES", "match"),
    ("minSampleSizeContinous", "minSampleSizeContinuous"),
    ("mod", "modulo"),
    ("modOrNull", "moduloOrNull"),
    ("caseWithoutExpr", "multiIf"),
    ("caseWithoutExpression", "multiIf"),
    ("ST_AsMVTGeom", "MVTEncodeGeom"),
    ("NATURAL_SORT_KEY", "naturalSortKey"),
    ("current_timestamp", "now"),
    ("localtimestamp", "now"),
    ("TO_UNIXTIME", "parseDateTime"),
    ("str_to_date", "parseDateTimeOrNull"),
    ("percentRank", "percent_rank"),
    ("instr", "positionCaseInsensitive"),
    ("pmod", "positiveModulo"),
    ("positive_modulo", "positiveModulo"),
    ("pmodOrNull", "positiveModuloOrNull"),
    ("positive_modulo_or_null", "positiveModuloOrNull"),
    ("power", "pow"),
    ("query_id", "queryID"),
    ("rand32", "rand"),
    ("ST_LineFromWKB", "readWKBLineString"),
    ("ST_MLineFromWKB", "readWKBMultiLineString"),
    ("ST_MPolyFromWKB", "readWKBMultiPolygon"),
    ("ST_PointFromWKB", "readWKBPoint"),
    ("ST_PolyFromWKB", "readWKBPolygon"),
    ("REGEXP_EXTRACT", "regexpExtract"),
    ("REGEXP_SUBSTR", "regexpExtract"),
    ("regexpInstr", "regexpPosition"),
    ("regexp_instr", "regexpPosition"),
    ("removeAccentsUTF8", "removeDiacriticsUTF8"),
    ("replace", "replaceAll"),
    ("REGEXP_REPLACE", "replaceRegexpAll"),
    ("rpad", "rightPad"),
    ("visitParamExtractBool", "simpleJSONExtractBool"),
    ("visitParamExtractFloat", "simpleJSONExtractFloat"),
    ("visitParamExtractInt", "simpleJSONExtractInt"),
    ("visitParamExtractRaw", "simpleJSONExtractRaw"),
    ("visitParamExtractString", "simpleJSONExtractString"),
    ("visitParamExtractUInt", "simpleJSONExtractUInt"),
    ("visitParamHas", "simpleJSONHas"),
    ("sqid", "sqidEncode"),
    ("byteSlice", "substring"),
    ("mid", "substring"),
    ("substr", "substring"),
    ("SUBSTRING_INDEX", "substringIndex"),
    ("timeSeriesTagsGroupToTags", "timeSeriesGroupToTags"),
    ("timeSeriesIdToTagsGroup", "timeSeriesIdToGroup"),
    ("curdate", "today"),
    ("current_date", "today"),
    ("DAY", "toDayOfMonth"),
    ("DAYOFMONTH", "toDayOfMonth"),
    ("DAYOFWEEK", "toDayOfWeek"),
    ("DAYOFYEAR", "toDayOfYear"),
    ("TO_DAYS", "toDaysSinceYearZero"),
    ("HOUR", "toHour"),
    ("LAST_DAY", "toLastDayOfMonth"),
    ("MICROSECOND", "toMicrosecond"),
    ("MILLISECOND", "toMillisecond"),
    ("MINUTE", "toMinute"),
    ("MONTH", "toMonth"),
    ("NANOSECOND", "toNanosecond"),
    ("QUARTER", "toQuarter"),
    ("SECOND", "toSecond"),
    ("toStartOfFiveMinute", "toStartOfFiveMinutes"),
    ("date_bin", "toStartOfInterval"),
    ("time_bucket", "toStartOfInterval"),
    ("to_utc_timestamp", "toUTCTimestamp"),
    ("week", "toWeek"),
    ("YEAR", "toYear"),
    ("yearweek", "toYearWeek"),
    ("trim", "trimBoth"),
    ("ltrim", "trimLeft"),
    ("rtrim", "trimRight"),
    ("truncate", "trunc"),
    ("vectorDifference", "tupleMinus"),
    ("vectorSum", "tuplePlus"),
    ("ucase", "upper"),
    ("UTC_timestamp", "UTCTimestamp"),
    ("width_bucket", "widthBucket"),
    ("anova", "analysisOfVariance"),
    ("any_value", "any"),
    ("any_value_respect_nulls", "any_respect_nulls"),
    ("anyRespectNulls", "any_respect_nulls"),
    ("anyValueRespectNulls", "any_respect_nulls"),
    ("first_value_respect_nulls", "any_respect_nulls"),
    ("firstValueRespectNulls", "any_respect_nulls"),
    ("anyLastRespectNulls", "anyLast_respect_nulls"),
    ("last_value_respect_nulls", "anyLast_respect_nulls"),
    ("lastValueRespectNulls", "anyLast_respect_nulls"),
    ("approx_top_count", "approx_top_k"),
    ("max_by", "argMax"),
    ("min_by", "argMin"),
    ("COVAR_POP", "covarPop"),
    ("COVAR_SAMP", "covarSamp"),
    ("array_agg", "groupArray"),
    ("BIT_AND", "groupBitAnd"),
    ("BIT_OR", "groupBitOr"),
    ("BIT_XOR", "groupBitXor"),
    ("group_concat", "groupConcat"),
    ("string_agg", "groupConcat"),
    ("lttb", "largestTriangleThreeBuckets"),
    ("ST_AsMVT", "MVTEncode"),
    ("median", "quantile"),
    ("medianBFloat16", "quantileBFloat16"),
    ("medianBFloat16Weighted", "quantileBFloat16Weighted"),
    ("medianDD", "quantileDD"),
    ("medianDeterministic", "quantileDeterministic"),
    ("medianExact", "quantileExact"),
    ("medianExactHigh", "quantileExactHigh"),
    ("medianExactLow", "quantileExactLow"),
    ("medianExactWeighted", "quantileExactWeighted"),
    (
        "medianExactWeightedInterpolated",
        "quantileExactWeightedInterpolated",
    ),
    ("medianGK", "quantileGK"),
    ("medianInterpolatedWeighted", "quantileInterpolatedWeighted"),
    ("medianTDigest", "quantileTDigest"),
    ("medianTDigestWeighted", "quantileTDigestWeighted"),
    ("medianTiming", "quantileTiming"),
    ("medianTimingWeighted", "quantileTimingWeighted"),
    ("STD", "stddevPop"),
    ("STDDEV_POP", "stddevPop"),
    ("STDDEV", "stddevSamp"),
    ("STDDEV_SAMP", "stddevSamp"),
    (
        "timeSeriesLastToGrid",
        "timeSeriesResampleToGridWithStaleness",
    ),
    ("VAR_POP", "varPop"),
    ("VAR_SAMP", "varSamp"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CatalogKind {
    Expression,
    Table,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectKind {
    Regular,
    Aggregate,
    Table,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Combinator {
    Array,
    Map,
    ForEach,
    Distinct,
    OrDefault,
    OrNull,
    If,
    Resample,
    SimpleState,
    State,
    Merge,
    MergeState,
}

impl Combinator {
    fn suffix(self) -> &'static str {
        match self {
            Self::Array => "Array",
            Self::Map => "Map",
            Self::ForEach => "ForEach",
            Self::Distinct => "Distinct",
            Self::OrDefault => "OrDefault",
            Self::OrNull => "OrNull",
            Self::If => "If",
            Self::Resample => "Resample",
            Self::SimpleState => "SimpleState",
            Self::State => "State",
            Self::Merge => "Merge",
            Self::MergeState => "MergeState",
        }
    }
}

fn names(kind: DirectKind) -> impl Iterator<Item = &'static str> {
    let source = match kind {
        DirectKind::Regular => REGULAR_NAMES,
        DirectKind::Aggregate => AGGREGATE_NAMES,
        DirectKind::Table => TABLE_NAMES,
    };
    source.split_ascii_whitespace()
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

fn cmp_ignore_ascii_case(left: &str, right: &str) -> Ordering {
    left.bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .cmp(right.bytes().map(|byte| byte.to_ascii_lowercase()))
        .then_with(|| left.cmp(right))
}

fn canonical_direct(name: &str) -> Option<(&'static str, DirectKind)> {
    for kind in [
        DirectKind::Regular,
        DirectKind::Aggregate,
        DirectKind::Table,
    ] {
        if let Some(canonical) = names(kind).find(|candidate| candidate.eq_ignore_ascii_case(name))
        {
            return Some((canonical, kind));
        }
    }
    let canonical = ALIASES
        .iter()
        .find(|(alias, _)| alias.eq_ignore_ascii_case(name))
        .map(|(_, canonical)| *canonical)?;
    for kind in [DirectKind::Regular, DirectKind::Aggregate] {
        if names(kind).any(|candidate| candidate == canonical) {
            return Some((canonical, kind));
        }
    }
    None
}

fn make(name: &str, parameter_groups: &[&[&'static str]]) -> BuiltinSignature {
    BuiltinSignature {
        name: name.to_string(),
        parameter_groups: parameter_groups
            .iter()
            .map(|group| group.to_vec())
            .collect(),
    }
}

fn regular_signatures(name: &str) -> Vec<BuiltinSignature> {
    match name {
        "arrayElement" => vec![make(name, &[&["array", "index"]])],
        "arrayFilter" => vec![make(name, &[&["lambda", "array", "...arrays"]])],
        "arrayJoin" => vec![make(name, &[&["array"]])],
        "arrayMap" => vec![make(name, &[&["lambda", "array", "...arrays"]])],
        "cityHash64" => vec![make(name, &[&["argument", "...arguments"]])],
        "concat" => vec![make(name, &[&["value", "...values"]])],
        "cume_dist" | "dense_rank" | "percent_rank" | "rank" | "row_number" => {
            vec![make(name, &[&[]])]
        }
        "dictGet" => vec![make(name, &[&["dictionary", "attribute", "key"]])],
        "formatDateTime" => vec![
            make(name, &[&["value", "format"]]),
            make(name, &[&["value", "format", "time_zone"]]),
        ],
        "geoDistance" => vec![make(
            name,
            &[&["longitude1", "latitude1", "longitude2", "latitude2"]],
        )],
        "JSONExtract" => vec![make(name, &[&["json", "path", "...paths", "return_type"]])],
        "JSONExtractString" => vec![make(name, &[&["json", "path", "...paths"]])],
        "lag" | "lagInFrame" | "lead" | "leadInFrame" => vec![
            make(name, &[&["value"]]),
            make(name, &[&["value", "offset"]]),
            make(name, &[&["value", "offset", "default"]]),
        ],
        "length" => vec![make(name, &[&["value"]])],
        "lower" | "upper" => vec![make(name, &[&["string"]])],
        "map" => vec![make(name, &[&["key", "value", "...pairs"]])],
        "now" => vec![make(name, &[&[]]), make(name, &[&["time_zone"]])],
        "ntile" => vec![make(name, &[&["buckets"]])],
        "substring" => vec![
            make(name, &[&["string", "offset"]]),
            make(name, &[&["string", "offset", "length"]]),
        ],
        "toDate" | "toDateTime" | "toStartOfDay" => vec![
            make(name, &[&["value"]]),
            make(name, &[&["value", "time_zone"]]),
        ],
        "toStartOfInterval" => vec![
            make(name, &[&["value", "INTERVAL x unit"]]),
            make(name, &[&["value", "INTERVAL x unit", "time_zone"]]),
            make(
                name,
                &[&["value", "INTERVAL x unit", "origin", "time_zone?"]],
            ),
        ],
        "tuple" => vec![make(name, &[&["value", "...values"]])],
        "URLHierarchy" => vec![make(name, &[&["url"]])],
        "buildId" | "currentDatabase" | "currentQueryID" | "currentUser" | "e"
        | "generateUUIDv4" | "generateUUIDv7" | "hostName" | "pi" | "rand" | "rand64"
        | "revision" | "serverTimezone" | "serverUUID" | "today" | "timezone" | "uptime"
        | "version" | "yesterday" => vec![make(name, &[&[]])],
        _ => vec![make(name, &[&["argument", "...arguments?"]])],
    }
}

fn aggregate_signatures(name: &str) -> Vec<BuiltinSignature> {
    match name {
        "count" => vec![make(name, &[&[]]), make(name, &[&["expression"]])],
        "sum" | "sumWithOverflow" | "avg" | "min" | "max" | "any" | "anyLast" | "anyHeavy" => {
            vec![make(name, &[&["value"]])]
        }
        "argMin" | "argMax" => vec![make(name, &[&["argument", "value"]])],
        "groupArray" | "groupUniqArray" => vec![
            make(name, &[&["expression"]]),
            make(name, &[&["max_size"], &["expression"]]),
        ],
        "groupArrayInsertAt" => vec![
            make(name, &[&["default_value?", "size?"]]),
            make(name, &[&["value", "position"]]),
        ],
        "groupConcat" => vec![
            make(name, &[&["delimiter?", "limit?"]]),
            make(name, &[&["expression"]]),
        ],
        "uniq" | "uniqExact" | "uniqHLL12" | "uniqTheta" => {
            vec![make(name, &[&["expression", "...expressions"]])]
        }
        "uniqCombined" | "uniqCombined64" => vec![
            make(name, &[&["HLL_precision?"]]),
            make(name, &[&["expression", "...expressions"]]),
        ],
        "quantile" | "quantileExact" | "quantileTDigest" | "quantileTiming"
        | "quantileBFloat16" => {
            vec![make(name, &[&["level?"]]), make(name, &[&["expression"]])]
        }
        "quantiles" | "quantilesExact" | "quantilesTDigest" | "quantilesTiming"
        | "quantilesBFloat16" => {
            vec![make(name, &[&["level", "...levels"], &["expression"]])]
        }
        "topK" => vec![
            make(name, &[&["N?", "load_factor?", "counts?"]]),
            make(name, &[&["expression"]]),
        ],
        "topKWeighted" => vec![
            make(name, &[&["N?", "load_factor?", "counts?"]]),
            make(name, &[&["expression", "weight"]]),
        ],
        "histogram" => vec![make(name, &[&["bins"]]), make(name, &[&["values"]])],
        "sequenceMatch" | "sequenceCount" => {
            vec![
                make(name, &[&["pattern"]]),
                make(name, &[&["timestamp", "...conditions"]]),
            ]
        }
        "windowFunnel" => vec![
            make(name, &[&["window", "mode?"]]),
            make(name, &[&["timestamp", "...conditions"]]),
        ],
        "retention" => vec![make(name, &[&["condition", "...conditions"]])],
        _ => vec![make(name, &[&["expression", "...expressions?"]])],
    }
}

fn table_signatures(name: &str) -> Vec<BuiltinSignature> {
    match name {
        "file" => vec![make(
            name,
            &[&["path", "format?", "structure?", "compression?"]],
        )],
        "s3" => vec![make(
            name,
            &[&["url", "format?", "structure?", "compression?"]],
        )],
        "mysql" => vec![make(
            name,
            &[&["address", "database", "table", "user", "password"]],
        )],
        "numbers" => vec![
            make(name, &[&["count"]]),
            make(name, &[&["offset", "count"]]),
            make(name, &[&["offset", "count", "step"]]),
        ],
        "postgresql" => vec![make(
            name,
            &[&[
                "address", "database", "table", "user", "password", "schema?",
            ]],
        )],
        "remote" => vec![make(
            name,
            &[&["addresses", "database", "table", "user?", "password?"]],
        )],
        "url" => vec![make(name, &[&["url", "format", "structure?", "headers?"]])],
        _ => vec![make(name, &[&["argument", "...arguments?"]])],
    }
}

fn direct_signatures(name: &str, kind: DirectKind) -> Vec<BuiltinSignature> {
    match kind {
        DirectKind::Regular => regular_signatures(name),
        DirectKind::Aggregate => aggregate_signatures(name),
        DirectKind::Table => table_signatures(name),
    }
}

fn combinator_sequences() -> &'static [Vec<Combinator>] {
    static SEQUENCES: OnceLock<Vec<Vec<Combinator>>> = OnceLock::new();
    SEQUENCES.get_or_init(|| {
        let mut sequences = Vec::new();
        let collections = [
            None,
            Some(Combinator::Array),
            Some(Combinator::Map),
            Some(Combinator::ForEach),
        ];
        let fallbacks = [None, Some(Combinator::OrDefault), Some(Combinator::OrNull)];
        let terminals = [
            None,
            Some(Combinator::SimpleState),
            Some(Combinator::State),
            Some(Combinator::Merge),
            Some(Combinator::MergeState),
        ];
        for collection in collections {
            for distinct in [false, true] {
                for fallback in fallbacks {
                    for conditional in [false, true] {
                        for terminal in terminals {
                            let mut sequence = Vec::new();
                            if let Some(collection) = collection {
                                sequence.push(collection);
                            }
                            if distinct {
                                sequence.push(Combinator::Distinct);
                            }
                            if let Some(fallback) = fallback {
                                sequence.push(fallback);
                            }
                            if conditional {
                                sequence.push(Combinator::If);
                            }
                            if let Some(terminal) = terminal {
                                sequence.push(terminal);
                            }
                            if !sequence.is_empty() {
                                sequences.push(sequence);
                            }
                        }
                    }
                }
            }
        }
        for conditional in [false, true] {
            for terminal in terminals {
                let mut sequence = vec![Combinator::Resample];
                if conditional {
                    sequence.push(Combinator::If);
                }
                if let Some(terminal) = terminal {
                    sequence.push(terminal);
                }
                sequences.push(sequence);
            }
        }
        sequences
    })
}

fn sequence_suffix(sequence: &[Combinator]) -> String {
    sequence.iter().map(|part| part.suffix()).collect()
}

fn array_parameter(index: usize) -> &'static str {
    match index {
        0 => "array",
        1 => "array_2",
        2 => "array_3",
        3 => "array_4",
        4 => "array_5",
        5 => "array_6",
        6 => "array_7",
        _ => "array_n",
    }
}

fn apply_combinator(signature: &mut BuiltinSignature, combinator: Combinator) {
    let Some(last_group) = signature.parameter_groups.last_mut() else {
        return;
    };
    match combinator {
        Combinator::Array | Combinator::ForEach => {
            for (index, parameter) in last_group.iter_mut().enumerate() {
                *parameter = if parameter.starts_with("...") {
                    "...arrays"
                } else {
                    array_parameter(index)
                };
            }
        }
        Combinator::Map => *last_group = vec!["map"],
        Combinator::If => last_group.push("condition"),
        Combinator::Resample => {
            last_group.push("resampling_key");
            let insert_at = signature.parameter_groups.len().saturating_sub(1);
            signature
                .parameter_groups
                .insert(insert_at, vec!["start", "end", "step"]);
        }
        Combinator::Merge | Combinator::MergeState => *last_group = vec!["state"],
        Combinator::Distinct
        | Combinator::OrDefault
        | Combinator::OrNull
        | Combinator::SimpleState
        | Combinator::State => {}
    }
}

fn generated_signatures(base: &'static str, sequence: &[Combinator]) -> Vec<BuiltinSignature> {
    let generated_name = format!("{base}{}", sequence_suffix(sequence));
    let mut signatures = aggregate_signatures(base);
    for signature in &mut signatures {
        signature.name.clone_from(&generated_name);
        for combinator in sequence {
            apply_combinator(signature, *combinator);
        }
    }
    signatures
}

fn generated_for_exact_name(name: &str) -> Vec<BuiltinSignature> {
    for base in names(DirectKind::Aggregate) {
        if name.len() <= base.len() || !starts_with_ignore_ascii_case(name, base) {
            continue;
        }
        let suffix = &name[base.len()..];
        for sequence in combinator_sequences() {
            if sequence_suffix(sequence).eq_ignore_ascii_case(suffix) {
                return generated_signatures(base, sequence);
            }
        }
    }
    Vec::new()
}

pub(crate) fn signatures_for(name: &str) -> Vec<BuiltinSignature> {
    if let Some((canonical, kind)) = canonical_direct(name) {
        return direct_signatures(canonical, kind);
    }
    generated_for_exact_name(name)
}

fn kind_matches(kind: DirectKind, requested: CatalogKind) -> bool {
    matches!(
        (kind, requested),
        (
            DirectKind::Regular | DirectKind::Aggregate,
            CatalogKind::Expression
        ) | (DirectKind::Table, CatalogKind::Table)
    )
}

fn aliases_match(canonical: &str, prefix: &str) -> bool {
    ALIASES
        .iter()
        .any(|(alias, target)| *target == canonical && starts_with_ignore_ascii_case(alias, prefix))
}

pub(crate) fn completion_catalog(
    prefix: &str,
    requested: CatalogKind,
    limit: usize,
) -> Vec<BuiltinSignature> {
    if limit == 0 {
        return Vec::new();
    }

    let kinds: &[DirectKind] = match requested {
        CatalogKind::Expression => &[DirectKind::Regular, DirectKind::Aggregate],
        CatalogKind::Table => &[DirectKind::Table],
    };
    let mut direct = kinds
        .iter()
        .flat_map(|kind| names(*kind).map(move |name| (name, *kind)))
        .filter(|(name, _)| {
            prefix.is_empty()
                || starts_with_ignore_ascii_case(name, prefix)
                || aliases_match(name, prefix)
        })
        .collect::<Vec<_>>();
    direct.sort_unstable_by(|(left, _), (right, _)| cmp_ignore_ascii_case(left, right));

    let mut results = direct
        .into_iter()
        .take(limit)
        .filter_map(|(name, kind)| direct_signatures(name, kind).into_iter().next())
        .collect::<Vec<_>>();
    if requested != CatalogKind::Expression || results.len() >= limit {
        return results;
    }

    for base in names(DirectKind::Aggregate) {
        if !prefix.is_empty()
            && !starts_with_ignore_ascii_case(base, prefix)
            && !starts_with_ignore_ascii_case(prefix, base)
        {
            continue;
        }
        for sequence in combinator_sequences() {
            let candidate = format!("{base}{}", sequence_suffix(sequence));
            if !starts_with_ignore_ascii_case(&candidate, prefix) {
                continue;
            }
            if let Some(signature) = generated_signatures(base, sequence).into_iter().next() {
                results.push(signature);
            }
            if results.len() == limit {
                return results;
            }
        }
    }
    results
}

pub(crate) fn contains_expression(name: &str) -> bool {
    canonical_direct(name).is_some_and(|(_, kind)| kind_matches(kind, CatalogKind::Expression))
        || !generated_for_exact_name(name).is_empty()
}

pub(crate) fn direct_expression_catalog() -> Vec<BuiltinSignature> {
    [DirectKind::Regular, DirectKind::Aggregate]
        .into_iter()
        .flat_map(|kind| {
            names(kind).filter_map(move |name| direct_signatures(name, kind).into_iter().next())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        completion_catalog, contains_expression, signatures_for, CatalogKind, AGGREGATE_NAMES,
        REGULAR_NAMES, TABLE_NAMES,
    };

    #[test]
    fn dbx_snapshot_counts_and_aliases_remain_aligned() {
        assert_eq!(REGULAR_NAMES.split_ascii_whitespace().count(), 1428);
        assert_eq!(AGGREGATE_NAMES.split_ascii_whitespace().count(), 173);
        assert_eq!(TABLE_NAMES.split_ascii_whitespace().count(), 90);
        assert_eq!(signatures_for("ucase")[0].name, "upper");
        assert!(contains_expression("array_agg"));
    }

    #[test]
    fn direct_overloads_and_parametric_groups_are_preserved() {
        assert_eq!(signatures_for("formatDateTime").len(), 2);
        assert_eq!(
            signatures_for("quantilesTDigest")[0].parameter_groups,
            vec![vec!["level", "...levels"], vec!["expression"]]
        );
        assert_eq!(signatures_for("numbers").len(), 3);
    }

    #[test]
    fn aggregate_combinators_transform_last_argument_group() {
        let signature = signatures_for("quantilesTDigestArrayIfState")
            .pop()
            .expect("generated combinator signature");
        assert_eq!(signature.name, "quantilesTDigestArrayIfState");
        assert_eq!(
            signature.parameter_groups,
            vec![vec!["level", "...levels"], vec!["array", "condition"]]
        );

        let resample = signatures_for("sumResampleIf")
            .pop()
            .expect("resample combinator signature");
        assert_eq!(
            resample.parameter_groups,
            vec![
                vec!["start", "end", "step"],
                vec!["value", "resampling_key", "condition"]
            ]
        );
    }

    #[test]
    fn completion_is_bounded_and_table_functions_are_separate() {
        let functions = completion_catalog("array", CatalogKind::Expression, 100);
        assert!(!functions.is_empty());
        assert!(functions.len() <= 100);
        assert!(functions
            .iter()
            .any(|signature| signature.name == "arrayMap"));
        assert!(!functions
            .iter()
            .any(|signature| signature.name == "arrowFlight"));

        let tables = completion_catalog("num", CatalogKind::Table, 100);
        assert!(tables.iter().any(|signature| signature.name == "numbers"));
        assert!(tables.iter().all(|signature| {
            TABLE_NAMES
                .split_ascii_whitespace()
                .any(|name| name == signature.name)
        }));
    }
}
